//! 文档模型与 tab runtime 的集中所有权。

use std::path::Path;

use appkit_core::workspace::types::TabId;

use crate::cursor_motion::{CursorContext, PageDirection};
use crate::edit_transaction::{build_edit_request, default_edit_plan, execute_edit_plan};
use crate::editor_runtime::{
    CloseConfirmation, DocumentTextEditError, DocumentTextReplacement, DocumentTextSnapshot,
    EditorDocumentSummary, EditorNotification, EditorOutcome, EditorTabSnapshot,
    EditorWorkspaceSnapshot, OpenDisposition, SemanticEditResult,
};
use crate::input_mapper::EditCommand;
use crate::prepared_tab::{PreparedTab, placeholder_display_entries};
use crate::tab_runtime::{TabRuntime, TabRuntimeStore};
use crate::tab_session::{TabSession, TabSessionMut};
use crate::view_route::ViewRouteTable;
use crate::workspace::{CloseTabDecision, Workspace, WorkspaceEffect};

pub(crate) struct ModelSession {
    workspace: Workspace,
    runtimes: TabRuntimeStore,
}

impl ModelSession {
    pub(crate) fn replace_parts(&mut self, workspace: Workspace, runtimes: TabRuntimeStore) {
        self.workspace = workspace;
        self.runtimes = runtimes;
        debug_assert_eq!(self.workspace.tab_ids(), self.runtimes.ids());
    }

    pub(crate) fn new(
        plugin_registry: ui::plugin::PluginRegistry,
        view_routes: ViewRouteTable,
    ) -> Self {
        Self {
            workspace: Workspace::with_plugins(plugin_registry, view_routes),
            runtimes: TabRuntimeStore::default(),
        }
    }

    pub(crate) fn install_prepared_tab(
        &mut self,
        prepared: PreparedTab,
        suggested_file_name: Option<String>,
        disposition: OpenDisposition,
    ) -> WorkspaceEffect {
        let effect = self.workspace.install_prepared_tab(
            &mut self.runtimes,
            prepared,
            suggested_file_name,
            disposition,
        );
        self.apply_workspace_effect(effect)
    }

    pub(crate) fn append_prepared_tab(
        &mut self,
        prepared: PreparedTab,
        suggested_file_name: Option<String>,
    ) -> TabId {
        let tab_id =
            self.workspace.append_prepared_tab(&mut self.runtimes, prepared, suggested_file_name);
        debug_assert_eq!(self.workspace.tab_ids(), self.runtimes.ids());
        tab_id
    }

    pub(crate) fn activate(&mut self, tab_id: TabId) -> Option<WorkspaceEffect> {
        let index = self.workspace.index_of(tab_id)?;
        let effect = self.workspace.switch_to(index);
        Some(self.apply_workspace_effect(effect))
    }

    pub(crate) fn close_decision(&self, tab_id: TabId) -> Option<CloseTabDecision> {
        let index = self.workspace.index_of(tab_id)?;
        Some(self.workspace.try_close_entry(index))
    }

    pub(crate) fn close(&mut self, tab_id: TabId) -> Option<WorkspaceEffect> {
        let index = self.workspace.index_of(tab_id)?;
        let effect = self
            .workspace
            .close_entry(index)
            .expect("close decision must be checked before closing a tab");
        Some(self.apply_workspace_effect(effect))
    }

    pub(crate) fn confirm_close(
        &mut self,
        tab_id: TabId,
        confirmation: CloseConfirmation,
    ) -> Option<WorkspaceEffect> {
        let decision = self.close_decision(tab_id)?;
        let should_close = match confirmation {
            CloseConfirmation::Saved => decision == CloseTabDecision::CanClose,
            CloseConfirmation::Discard => decision != CloseTabDecision::Pinned,
            CloseConfirmation::Cancel => false,
        };
        should_close.then(|| self.close(tab_id)).flatten()
    }

    pub(crate) fn active_tab_id(&self) -> Option<TabId> {
        self.workspace.active_tab_id()
    }

    pub(crate) fn active_selected_text(&self) -> Option<String> {
        let tab_id = self.workspace.active_tab_id()?;
        let tab = self.tab_session(tab_id)?;
        if tab.plugin_name() == ui::plugin::PLUGIN_MARKDOWN_EDITOR {
            let reading_text = tab.selected_text();
            return (!reading_text.is_empty()).then_some(reading_text);
        }
        self.workspace
            .active_entry()?
            .extract_selected_text()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    pub(crate) fn tab_index(&self, tab_id: TabId) -> Option<usize> {
        self.workspace.index_of(tab_id)
    }

    pub(crate) fn tab_id_at(&self, index: usize) -> Option<TabId> {
        self.workspace.tab_id_at(index)
    }

    pub(crate) fn tab_count(&self) -> usize {
        self.workspace.len()
    }

    pub(crate) fn tab_ids_in_order(&self) -> Vec<TabId> {
        self.workspace.tab_indices().filter_map(|index| self.workspace.tab_id_at(index)).collect()
    }

    pub(crate) fn runtime_tab_ids(&self) -> std::collections::HashSet<TabId> {
        self.runtimes.ids()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.workspace.is_empty()
    }

    pub(crate) fn is_pinned(&self, tab_id: TabId) -> bool {
        self.workspace.is_pinned_id(tab_id)
    }

    pub(crate) fn tab_title(&self, tab_id: TabId) -> Option<String> {
        let index = self.workspace.index_of(tab_id)?;
        self.workspace.entry_title(index)
    }

    pub(crate) fn clear_suggested_file_name(&mut self, tab_id: TabId) {
        if let Some(index) = self.workspace.index_of(tab_id) {
            self.workspace.clear_suggested_file_name(index);
        }
    }

    pub(crate) fn tab_session(&self, tab_id: TabId) -> Option<TabSession<'_>> {
        let index = self.workspace.index_of(tab_id)?;
        let document = self.workspace.entry(index)?;
        let runtime = self.runtimes.get(tab_id)?;
        Some(TabSession::new(tab_id, document, runtime))
    }

    pub(crate) fn tab_session_mut(&mut self, tab_id: TabId) -> Option<TabSessionMut<'_>> {
        let index = self.workspace.index_of(tab_id)?;
        let runtime = self.runtimes.get_mut(tab_id)?;
        let document = self.workspace.entry_mut(index)?;
        Some(TabSessionMut::new(tab_id, document, runtime))
    }

    pub(crate) fn tab_runtime(&self, tab_id: TabId) -> Option<&TabRuntime> {
        self.runtimes.get(tab_id)
    }

    pub(crate) fn tab_runtime_mut(&mut self, tab_id: TabId) -> Option<&mut TabRuntime> {
        self.runtimes.get_mut(tab_id)
    }

    pub(crate) fn tab_id_for_path(&self, path: &Path) -> Option<TabId> {
        self.workspace.find_by_path(path).and_then(|index| self.workspace.tab_id_at(index))
    }

    pub(crate) fn document_summary(&self, tab_id: TabId) -> Option<EditorDocumentSummary> {
        let document = self.workspace.entry_by_id(tab_id)?;
        Some(EditorDocumentSummary {
            tab_id,
            path: document.file_path.clone(),
            dirty: document.dirty,
            content_revision: document.content_revision(),
            disk_revision: document.disk_revision.clone(),
            pinned: self.workspace.is_pinned_id(tab_id),
        })
    }

    pub(crate) fn document_text_snapshot(&self, tab_id: TabId) -> Option<DocumentTextSnapshot> {
        let document = self.workspace.entry_by_id(tab_id)?;
        Some(DocumentTextSnapshot {
            tab_id,
            text: document.full_text(),
            content_revision: document.content_revision(),
        })
    }

    pub(crate) fn replace_document_text(
        &mut self,
        request: DocumentTextReplacement,
        line_height: f32,
    ) -> Result<EditorOutcome, DocumentTextEditError> {
        let Some(previous_summary) = self.document_summary(request.tab_id) else {
            return Err(DocumentTextEditError::UnknownTab { tab_id: request.tab_id });
        };
        let actual_revision = previous_summary.content_revision;
        if request.content_revision != actual_revision {
            return Err(DocumentTextEditError::StaleRevision {
                expected: request.content_revision,
                actual: actual_revision,
            });
        }
        let (source_generation, cursor_after) = {
            let Some(tab) = self.tab_session(request.tab_id) else {
                return Err(DocumentTextEditError::UnknownTab { tab_id: request.tab_id });
            };
            if !tab.allows_editing() {
                return Err(DocumentTextEditError::ReadOnly { tab_id: request.tab_id });
            }
            let text_length = tab.document.buffer_len();
            if request.range.start > request.range.end
                || request.range.end > text_length
                || !valid_document_boundary(tab.document, request.range.start)
                || !valid_document_boundary(tab.document, request.range.end)
            {
                return Err(DocumentTextEditError::InvalidByteRange {
                    range: request.range,
                    text_length,
                });
            }
            (tab.document.generation(), request.range.start + request.replacement.len())
        };
        let plan = ui::plugin::EditPlan::Apply(ui::plugin::EditTransaction::replace(
            source_generation,
            request.range,
            request.replacement,
            cursor_after,
        ));
        self.apply_session_edit(previous_summary.tab_id, plan, line_height).ok_or(
            DocumentTextEditError::InvalidByteRange {
                range: cursor_after..cursor_after,
                text_length: self
                    .tab_session(previous_summary.tab_id)
                    .map_or(0, |tab| tab.document.buffer_len()),
            },
        )
    }

    pub(crate) fn edit_active_document(
        &mut self,
        intent: ui::plugin::EditIntent,
        line_height: f32,
    ) -> EditorOutcome {
        let Some(tab_id) = self.active_tab_id() else {
            return EditorOutcome::default();
        };
        if !self.tab_session(tab_id).is_some_and(|tab| tab.allows_editing()) {
            return EditorOutcome::default();
        }
        self.synchronize_plugin_source_if_stale(tab_id);
        let Some(tab) = self.tab_session(tab_id) else {
            return EditorOutcome::default();
        };
        let plan = resolve_edit_plan(&tab, intent);
        self.apply_session_edit(tab_id, plan, line_height).unwrap_or_default()
    }

    pub(crate) fn execute_source_command(
        &mut self,
        command: &EditCommand,
        line_height: f32,
    ) -> EditorOutcome {
        let Some(tab_id) = self.active_tab_id() else {
            return EditorOutcome::default();
        };
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return EditorOutcome::default();
        };
        if !tab.allows_editing() {
            return EditorOutcome::default();
        }
        let previous_revision = tab.document.content_revision();
        let previous_dirty = tab.document.dirty;
        let advance_cache = tab.take_advance_cache();
        let mut presentation = tab.take_presentation();
        let page_step = presentation.display.viewport.visible_rows.saturating_sub(1).max(1);
        let edit = crate::editor_commands::execute_edit_command_with_system_clipboard(
            command,
            tab.document,
            &advance_cache,
            &mut presentation.cursor_render_state,
            page_step,
        );
        tab.restore_presentation(presentation);
        tab.restore_advance_cache(advance_cache);
        if !edit.executed {
            return EditorOutcome::default();
        }
        refresh_transaction_presentation(&mut tab, &edit, line_height);
        if previous_revision != tab.document.content_revision() {
            synchronize_plugin_source(&mut tab);
        }
        let mut outcome = edit_outcome(
            tab_id,
            previous_revision,
            previous_dirty,
            tab.document.content_revision(),
            tab.document.dirty,
        );
        if crate::editor_commands::edit_requires_reshape(command, &edit) {
            outcome.shell_effect = outcome.shell_effect.merge(crate::event::ShellEffect::RESHAPE);
        }
        outcome
    }

    pub(crate) fn apply_active_edit_transaction(
        &mut self,
        transaction: ui::plugin::EditTransaction,
        line_height: f32,
    ) -> EditorOutcome {
        self.active_tab_id()
            .and_then(|tab_id| {
                self.apply_session_edit(
                    tab_id,
                    ui::plugin::EditPlan::Apply(transaction),
                    line_height,
                )
            })
            .unwrap_or_default()
    }

    fn apply_session_edit(
        &mut self,
        tab_id: TabId,
        plan: ui::plugin::EditPlan,
        line_height: f32,
    ) -> Option<EditorOutcome> {
        let mut tab = self.tab_session_mut(tab_id)?;
        if !tab.allows_editing() {
            return None;
        }
        let previous_revision = tab.document.content_revision();
        let previous_dirty = tab.document.dirty;
        let previous_anchor = tab.document.cursor().selection_anchor;
        let edit = execute_edit_plan(plan, tab.document).ok()?;
        let selection_changed = previous_anchor != tab.document.cursor().selection_anchor;
        if !edit.edit_outcome.executed && !edit.cursor_moved && !selection_changed {
            return Some(EditorOutcome::default());
        }
        refresh_transaction_presentation(&mut tab, &edit.edit_outcome, line_height);
        if edit.content_revision != previous_revision {
            synchronize_plugin_source(&mut tab);
        } else {
            synchronize_plugin_caret(&mut tab);
        }
        let mut outcome = edit_outcome(
            tab_id,
            previous_revision,
            previous_dirty,
            edit.content_revision,
            edit.dirty,
        );
        if edit.edit_outcome.executed
            && (edit.edit_outcome.new_line_count != edit.edit_outcome.old_line_count
                || edit.edit_outcome.dirty_lines.is_some_and(|range| range.len() > 1))
        {
            outcome.shell_effect = outcome.shell_effect.merge(crate::event::ShellEffect::RESHAPE);
        }
        Some(outcome)
    }

    pub(crate) fn navigate_active_document(
        &mut self,
        key: ui::KeyCode,
        modifiers: ui::core::Modifiers,
        line_height: f32,
    ) -> EditorOutcome {
        let Some(tab_id) = self.active_tab_id() else {
            return EditorOutcome::default();
        };
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return EditorOutcome::default();
        };
        let document = &mut tab.document;
        match (key, modifiers.shift, modifiers.cmd || modifiers.ctrl, modifiers.alt) {
            (ui::KeyCode::Left, true, _, true) => document.extend_selection_word_left(),
            (ui::KeyCode::Right, true, _, true) => document.extend_selection_word_right(),
            (ui::KeyCode::Left, true, true, _) => document.extend_selection_to_line_start(),
            (ui::KeyCode::Right, true, true, _) => document.extend_selection_to_line_end(),
            (ui::KeyCode::Up, true, true, _) => document.extend_selection_to_doc_start(),
            (ui::KeyCode::Down, true, true, _) => document.extend_selection_to_doc_end(),
            (ui::KeyCode::Left, true, _, _) => document.extend_selection_left(),
            (ui::KeyCode::Right, true, _, _) => document.extend_selection_right(),
            (ui::KeyCode::Up, true, _, _) => document.extend_selection_up(),
            (ui::KeyCode::Down, true, _, _) => document.extend_selection_down(),
            (ui::KeyCode::Left, false, _, true) => document.cursor_move_word_left(),
            (ui::KeyCode::Right, false, _, true) => document.cursor_move_word_right(),
            (ui::KeyCode::Left | ui::KeyCode::Home, false, true, _) => {
                document.cursor_move_to_line_start()
            }
            (ui::KeyCode::Right | ui::KeyCode::End, false, true, _) => {
                document.cursor_move_to_line_end()
            }
            (ui::KeyCode::Up, false, true, _) => document.cursor_move_to_offset(0),
            (ui::KeyCode::Down, false, true, _) => {
                document.cursor_move_to_offset(document.buffer_len())
            }
            (ui::KeyCode::Left, false, _, _) => document.cursor_move_left(),
            (ui::KeyCode::Right, false, _, _) => document.cursor_move_right(),
            (ui::KeyCode::Up, false, _, _) => document.cursor_move_up(),
            (ui::KeyCode::Down, false, _, _) => document.cursor_move_down(),
            (ui::KeyCode::Home, false, _, _) => document.cursor_move_to_line_start(),
            (ui::KeyCode::End, false, _, _) => document.cursor_move_to_line_end(),
            (ui::KeyCode::PageUp, false, _, _) => tab.page_up(line_height),
            (ui::KeyCode::PageDown, false, _, _) => tab.page_down(line_height),
            _ => return EditorOutcome::default(),
        }
        // Keyboard navigation is user-driven caret movement: split any ongoing
        // undo coalescing run so the next edit starts a fresh undo entry.
        tab.document.break_edit_merge();
        tab.cursor_render_state_mut().cursor_blink_instant = std::time::Instant::now();
        if tab.handles_own_rendering() {
            tab.invalidate_cursor_visibility();
        } else {
            tab.ensure_cursor_visible(line_height);
        }
        EditorOutcome {
            shell_effect: crate::event::ShellEffect::REDRAW,
            ..EditorOutcome::default()
        }
    }

    pub(crate) fn page_active_document(
        &mut self,
        direction: PageDirection,
        line_height: f32,
    ) -> EditorOutcome {
        let Some(mut tab) = self.active_tab_id().and_then(|tab_id| self.tab_session_mut(tab_id))
        else {
            return EditorOutcome::default();
        };
        match direction {
            PageDirection::Up => tab.page_up(line_height),
            PageDirection::Down => tab.page_down(line_height),
        }
        EditorOutcome {
            shell_effect: crate::event::ShellEffect::REDRAW,
            ..EditorOutcome::default()
        }
    }

    pub(crate) fn move_active_visual_cursor(
        &mut self,
        delta: isize,
        extend_selection: bool,
        dpi_scale: f32,
        line_height: f32,
        frame_cache: &crate::frame_cache::FrameCache,
    ) -> EditorOutcome {
        let Some(mut tab) = self.active_tab_id().and_then(|tab_id| self.tab_session_mut(tab_id))
        else {
            return EditorOutcome::default();
        };
        let display_map = tab.display_map_clone();
        let previous_visual_line = tab.cursor_visual_line();
        let previous_document_line = tab.document.cursor_line();
        let advance_cache = tab.take_advance_cache();
        let context = CursorContext {
            display_map: &display_map,
            cursor_visual_line: previous_visual_line,
            advance_cache: &advance_cache,
            first_line: &frame_cache.first_line,
            last_line: &frame_cache.last_line,
            visible_rows: tab.visible_rows(),
            first_visible_row: tab.display().viewport.first_visible_row(),
            scroll_top: tab.scroll_top(),
            sticky_x: tab.cursor_render_state().sticky_x,
            dpi_scale,
        };
        if extend_selection {
            tab.document.ensure_selection_active();
        } else {
            tab.document.cursor_mut().selection_anchor = None;
        }
        tab.move_cursor_visual(delta, context);
        tab.restore_advance_cache(advance_cache);
        tab.ensure_cursor_visible(line_height);
        // A wrapped line can extend past the viewport without crossing a document line.
        if delta > 0
            && previous_document_line == tab.document.cursor_line()
            && previous_visual_line.is_some_and(|line| line + 1 >= tab.visible_rows())
        {
            tab.display_mut().viewport.scroll_pixels(line_height, &display_map, line_height);
            tab.refresh_scroll_metrics(line_height);
        }
        EditorOutcome {
            shell_effect: crate::event::ShellEffect::REDRAW,
            ..EditorOutcome::default()
        }
    }

    pub(crate) fn select_all_active_document(&mut self) -> EditorOutcome {
        let Some(tab_id) = self.active_tab_id() else {
            return EditorOutcome::default();
        };
        let Some(tab) = self.tab_session_mut(tab_id) else {
            return EditorOutcome::default();
        };
        tab.document.select_all();
        EditorOutcome {
            shell_effect: crate::event::ShellEffect::REDRAW,
            ..EditorOutcome::default()
        }
    }

    pub(crate) fn undo_or_redo_active_document(
        &mut self,
        redo: bool,
        line_height: f32,
    ) -> EditorOutcome {
        let Some(tab_id) = self.active_tab_id() else {
            return EditorOutcome::default();
        };
        let Some(previous_summary) = self.document_summary(tab_id) else {
            return EditorOutcome::default();
        };
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return EditorOutcome::default();
        };
        if !tab.allows_editing() {
            return EditorOutcome::default();
        }
        if redo {
            tab.document.redo();
        } else {
            tab.document.undo();
        }
        refresh_presentation_after_edit(&mut tab, line_height);
        synchronize_plugin_source(&mut tab);
        edit_outcome(
            tab_id,
            previous_summary.content_revision,
            previous_summary.dirty,
            tab.document.content_revision(),
            tab.document.dirty,
        )
    }

    pub(crate) fn scroll_active_document(
        &mut self,
        pixels: f32,
        plugin_viewport_height: f32,
        line_height: f32,
    ) -> EditorOutcome {
        if pixels == 0.0 {
            return EditorOutcome::default();
        }
        if self.scroll_active_plugin(pixels, plugin_viewport_height) {
            return EditorOutcome {
                shell_effect: crate::event::ShellEffect::REDRAW,
                ..EditorOutcome::default()
            };
        }
        if !self.scroll_active_text(pixels, line_height) {
            return EditorOutcome::default();
        }
        EditorOutcome {
            shell_effect: crate::event::ShellEffect::REDRAW
                .merge(crate::event::ShellEffect::RESHAPE),
            ..EditorOutcome::default()
        }
    }

    pub(crate) fn scroll_active_plugin(&mut self, pixels: f32, viewport_height: f32) -> bool {
        self.active_tab_id().and_then(|tab_id| self.tab_session_mut(tab_id)).is_some_and(
            |mut tab| {
                tab.handles_own_rendering()
                    && tab.send_message(ui::plugin::PluginMessage::Scroll {
                        delta: pixels,
                        viewport_h: viewport_height,
                    })
            },
        )
    }

    pub(crate) fn scroll_active_text(&mut self, pixels: f32, line_height: f32) -> bool {
        let Some(mut tab) = self.active_tab_id().and_then(|tab_id| self.tab_session_mut(tab_id))
        else {
            return false;
        };
        tab.scroll_viewport_by_pixels(pixels, line_height);
        true
    }

    pub(crate) fn execute_semantic_edit(
        &mut self,
        command: ui::plugin::SemanticEditCommand,
        line_height: f32,
    ) -> (SemanticEditResult, EditorOutcome) {
        let Some(tab_id) = self.active_tab_id() else {
            return (SemanticEditResult::NoChange, EditorOutcome::default());
        };
        let Some(previous_summary) = self.document_summary(tab_id) else {
            return (SemanticEditResult::NoChange, EditorOutcome::default());
        };
        if !self.tab_session(tab_id).is_some_and(|tab| tab.allows_editing()) {
            return (SemanticEditResult::NoChange, EditorOutcome::default());
        }
        if matches!(
            command,
            ui::plugin::SemanticEditCommand::Undo | ui::plugin::SemanticEditCommand::Redo
        ) {
            let redo = matches!(command, ui::plugin::SemanticEditCommand::Redo);
            let outcome = self.undo_or_redo_active_document(redo, line_height);
            let changed = self.document_summary(tab_id).is_some_and(|summary| {
                summary.content_revision != previous_summary.content_revision
            });
            return (
                if changed { SemanticEditResult::Applied } else { SemanticEditResult::NoChange },
                outcome,
            );
        }

        let structural_intent = match &command {
            ui::plugin::SemanticEditCommand::PromoteObject => {
                Some(ui::plugin::EditIntent::PromoteObject)
            }
            ui::plugin::SemanticEditCommand::DemoteObject => {
                Some(ui::plugin::EditIntent::DemoteObject)
            }
            _ => None,
        };
        if let Some(intent) = structural_intent {
            let Some(tab) = self.tab_session(tab_id) else {
                return (SemanticEditResult::NoChange, EditorOutcome::default());
            };
            let plan = resolve_edit_plan(&tab, intent);
            return self.apply_semantic_edit_plan(tab_id, plan, line_height);
        }

        let plan = {
            let Some(tab) = self.tab_session(tab_id) else {
                return (SemanticEditResult::NoChange, EditorOutcome::default());
            };
            let selection = tab
                .document
                .selection_range()
                .and_then(|(start, end)| (start < end).then_some(start..end));
            let query = ui::plugin::PluginQuery::PlanSemanticEdit {
                command,
                source_generation: tab.document.generation(),
                cursor_byte: tab.document.cursor_offset().to_usize(),
                selection,
            };
            match tab.query(query) {
                ui::plugin::PluginResponse::SemanticEdit(plan) => plan,
                _ => ui::plugin::SemanticEditPlan::Unsupported,
            }
        };
        let ui::plugin::SemanticEditPlan::Apply(transaction) = plan else {
            return (
                match plan {
                    ui::plugin::SemanticEditPlan::NoChange => SemanticEditResult::NoChange,
                    ui::plugin::SemanticEditPlan::Unsupported
                    | ui::plugin::SemanticEditPlan::Apply(_) => SemanticEditResult::Unsupported,
                },
                EditorOutcome::default(),
            );
        };
        self.apply_semantic_edit_plan(tab_id, ui::plugin::EditPlan::Apply(transaction), line_height)
    }

    pub(crate) fn apply_active_mindmap_theme(
        &mut self,
        theme_id: String,
        line_height: f32,
    ) -> EditorOutcome {
        let Some(tab_id) = self.active_tab_id() else {
            return EditorOutcome::default();
        };
        let plan = {
            let Some(tab) = self.tab_session(tab_id) else {
                return EditorOutcome::default();
            };
            if tab.plugin_name() != ui::plugin::PLUGIN_MINDMAP || !tab.allows_editing() {
                return EditorOutcome::default();
            }
            let source_generation = tab.document.generation();
            match tab
                .query(ui::plugin::PluginQuery::PlanMindmapTheme { theme_id, source_generation })
            {
                ui::plugin::PluginResponse::EditPlan(plan) => Some(plan),
                _ => None,
            }
        };
        let Some(plan) = plan else {
            return EditorOutcome::default();
        };
        self.apply_session_edit(tab_id, plan, line_height).unwrap_or_default()
    }

    fn apply_semantic_edit_plan(
        &mut self,
        tab_id: TabId,
        plan: ui::plugin::EditPlan,
        line_height: f32,
    ) -> (SemanticEditResult, EditorOutcome) {
        let outcome = self.apply_session_edit(tab_id, plan, line_height).unwrap_or_default();
        let result = if outcome.shell_effect == crate::event::ShellEffect::NONE {
            SemanticEditResult::NoChange
        } else {
            SemanticEditResult::Applied
        };
        (result, outcome)
    }

    pub(crate) fn document_save_snapshot(
        &self,
        tab_id: TabId,
    ) -> Option<(std::path::PathBuf, Vec<u8>, Option<appkit_core::file_safety::DiskRevision>, u64)>
    {
        let document = self.workspace.entry_by_id(tab_id)?;
        let path = document.file_path.clone()?;
        Some((
            path,
            document.serialized_contents_for_save(),
            document.disk_revision.clone(),
            document.content_revision(),
        ))
    }

    pub(crate) fn document_save_snapshot_as(
        &self,
        tab_id: TabId,
        path: &Path,
    ) -> Option<(Vec<u8>, Option<appkit_core::file_safety::DiskRevision>, u64)> {
        let document = self.workspace.entry_by_id(tab_id)?;
        let expected_revision = (document.file_path.as_deref() == Some(path))
            .then_some(document.disk_revision.clone())
            .flatten();
        Some((
            document.serialized_contents_for_save(),
            expected_revision,
            document.content_revision(),
        ))
    }

    pub(crate) fn apply_save_completion(
        &mut self,
        tab_id: TabId,
        path: std::path::PathBuf,
        content_revision: u64,
        disk_revision: appkit_core::file_safety::DiskRevision,
    ) -> Option<(bool, bool)> {
        self.workspace.apply_save_completion(tab_id, path, content_revision, disk_revision)
    }

    pub(crate) fn replace_document(
        &mut self,
        tab_id: TabId,
        document: appkit_core::document::DocumentModel,
    ) -> bool {
        let Some(index) = self.workspace.index_of(tab_id) else {
            return false;
        };
        let Some(current) = self.workspace.entry_doc_mut(index) else {
            return false;
        };
        *current = document;
        true
    }

    pub(crate) fn update_document_path(
        &mut self,
        tab_id: TabId,
        path: std::path::PathBuf,
        disk_revision: Option<appkit_core::file_safety::DiskRevision>,
    ) -> bool {
        let Some(index) = self.workspace.index_of(tab_id) else {
            return false;
        };
        let Some(document) = self.workspace.entry_doc_mut(index) else {
            return false;
        };
        document.file_path = Some(path.clone());
        document.disk_revision = disk_revision;
        document.set_language_from_path(&path);
        true
    }

    pub(crate) fn detach_document(
        &mut self,
        tab_id: TabId,
        suggested_file_name: Option<String>,
        dirty_snapshot_id: Option<String>,
    ) -> bool {
        let Some(index) = self.workspace.index_of(tab_id) else {
            return false;
        };
        let Some(document) = self.workspace.entry_doc_mut(index) else {
            return false;
        };
        document.file_path = None;
        document.disk_revision = None;
        document.dirty = true;
        if document.dirty_snapshot_id.is_none() {
            document.dirty_snapshot_id = dirty_snapshot_id;
        }
        self.workspace.set_suggested_file_name(index, suggested_file_name);
        true
    }

    pub(crate) fn document_summaries(&self) -> Vec<EditorDocumentSummary> {
        self.workspace
            .entries()
            .iter()
            .filter_map(|entry| self.document_summary(entry.id))
            .collect()
    }

    pub(crate) fn workspace_snapshot(&self) -> EditorWorkspaceSnapshot {
        let tabs = self
            .workspace
            .entries()
            .iter()
            .map(|entry| {
                let document = &entry.value;
                let session = crate::tab_session::TabSession::new(
                    entry.id,
                    document,
                    self.runtimes
                        .get(entry.id)
                        .expect("every workspace entry must have a matching tab runtime"),
                );
                let scroll_anchor = session.scroll_anchor_state();
                let preview_anchor_text = if session.allows_editing() {
                    None
                } else {
                    session.scroll_anchor().map(|(text, _)| text)
                };
                let content_lines = (0..document.line_count())
                    .filter_map(|line| {
                        document
                            .doc_line_bytes(line)
                            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    })
                    .collect();
                let clean_untitled_content =
                    (document.file_path.is_none() && !document.dirty).then(|| document.full_text());
                let default_plugin_name = document
                    .file_path
                    .as_deref()
                    .and_then(|path| self.workspace.plugin_route_for_path(path))
                    .map(|route| route.default_plugin.to_owned());

                EditorTabSnapshot {
                    tab_id: entry.id,
                    path: document.file_path.clone(),
                    suggested_file_name: entry.suggested_file_name.clone(),
                    cursor_offset: document
                        .cursor()
                        .snapshot_offset
                        .unwrap_or(document.cursor().offset.to_usize()),
                    selection_anchor: document
                        .cursor()
                        .snapshot_selection_anchor
                        .unwrap_or(document.cursor().selection_anchor),
                    cursor_line: document.cursor_line(),
                    cursor_column: document.cursor_column(),
                    dirty: document.dirty,
                    disk_revision: document.disk_revision.clone(),
                    dirty_snapshot_id: document.dirty_snapshot_id.clone(),
                    scroll_anchor_line: scroll_anchor.doc_line,
                    scroll_anchor_offset: scroll_anchor.pixel_offset,
                    preview_anchor_text,
                    plugin_name: session.plugin_name().to_owned(),
                    default_plugin_name,
                    allows_editing: session.allows_editing(),
                    content_lines,
                    clean_untitled_content,
                }
            })
            .collect();

        EditorWorkspaceSnapshot { active_index: self.workspace.active_index(), tabs }
    }

    pub(crate) fn has_back_history(&self) -> bool {
        self.workspace.has_back_history()
    }

    pub(crate) fn has_forward_history(&self) -> bool {
        self.workspace.has_forward_history()
    }

    pub(crate) fn toggle_target(&self) -> Option<&'static str> {
        self.workspace.toggle_target()
    }

    pub(crate) fn toggle_pin(
        &mut self,
        tab_id: TabId,
    ) -> Option<appkit_core::navigator::NavEffect> {
        let index = self.workspace.index_of(tab_id)?;
        Some(self.workspace.toggle_pin_at(index))
    }

    pub(crate) fn toggle_active_pin(&mut self) -> appkit_core::navigator::NavEffect {
        self.workspace.toggle_pin()
    }

    pub(crate) fn navigate_back(&mut self) -> appkit_core::navigator::NavEffect {
        self.workspace.go_back()
    }

    pub(crate) fn navigate_forward(&mut self) -> appkit_core::navigator::NavEffect {
        self.workspace.go_forward()
    }

    pub(crate) fn upgrade_active_preview(&mut self) -> appkit_core::navigator::NavEffect {
        self.workspace.upgrade_preview_if_needed()
    }

    pub(crate) fn switch_active_plugin(&mut self) {
        self.workspace.switch_plugin_with_runtime(&mut self.runtimes);
        debug_assert_eq!(self.workspace.tab_ids(), self.runtimes.ids());
    }

    pub(crate) fn active_is_toggled(&self, plugin_name: &str) -> bool {
        self.workspace.is_toggled_for_plugin(plugin_name)
    }

    pub(crate) fn pinned_paths(&self) -> Vec<std::path::PathBuf> {
        self.workspace.pinned_paths()
    }

    pub(crate) fn restore_pinned(&mut self, paths: &[std::path::PathBuf]) {
        self.workspace.restore_pinned(paths);
    }

    pub(crate) fn create_plugin_for_path(&self, path: &Path) -> Box<dyn ui::plugin::ViewPlugin> {
        self.workspace.create_plugin_for_path(path)
    }

    pub(crate) fn create_plugin_by_name(
        &self,
        plugin_name: &str,
    ) -> Box<dyn ui::plugin::ViewPlugin> {
        self.workspace.create_plugin_by_name(plugin_name)
    }

    fn apply_workspace_effect(&mut self, effect: WorkspaceEffect) -> WorkspaceEffect {
        self.reconcile_runtime_store(&effect);
        let activated_tab_id = match effect {
            WorkspaceEffect::Activated(tab_id) => Some(tab_id),
            WorkspaceEffect::Closed { activated, .. } => activated,
            WorkspaceEffect::None => None,
        };
        if let Some(tab_id) = activated_tab_id {
            self.synchronize_plugin_source(tab_id);
        }
        effect
    }

    fn reconcile_runtime_store(&mut self, effect: &WorkspaceEffect) {
        effect.reconcile_runtime_store(&mut self.runtimes);
        debug_assert_eq!(self.workspace.tab_ids(), self.runtimes.ids());
    }

    fn synchronize_plugin_source(&mut self, tab_id: TabId) {
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return;
        };
        synchronize_plugin_source(&mut tab);
    }

    fn synchronize_plugin_source_if_stale(&mut self, tab_id: TabId) {
        let Some(mut tab) = self.tab_session_mut(tab_id) else {
            return;
        };
        let source_generation = tab.document.generation();
        if !tab.needs_source_update(source_generation) {
            return;
        }
        synchronize_plugin_source(&mut tab);
    }
}

fn resolve_edit_plan(tab: &TabSession<'_>, intent: ui::plugin::EditIntent) -> ui::plugin::EditPlan {
    let request = build_edit_request(tab.document, intent);
    match tab.plan_edit_request(&request) {
        ui::plugin::EditPlan::UseDefault => default_edit_plan(&request, tab.document),
        plan => plan,
    }
}

fn valid_document_boundary(document: &appkit_core::document::DocumentModel, byte: usize) -> bool {
    if byte > document.buffer_len() {
        return false;
    }
    let source = document.full_text();
    textora_core::unicode::CursorNav::new(&source)
        .goto_byte(textora_core::types::ByteIndex(byte))
        .offset
        == textora_core::types::ByteIndex(byte)
}

fn refresh_transaction_presentation(
    tab: &mut TabSessionMut<'_>,
    edit: &crate::edit_transaction::EditOutcome,
    line_height: f32,
) {
    if edit.executed
        && let Some(dirty_lines) = &edit.dirty_lines
    {
        if edit.invalidates_all_render_cache() {
            let changed_line_end =
                (dirty_lines.end + edit.new_line_count).saturating_sub(edit.old_line_count);
            let entries = (dirty_lines.start..changed_line_end)
                .map(|line| {
                    let offset = tab.document.line_byte_offset(line).unwrap_or(0);
                    let length = tab.document.line_byte_length(line).unwrap_or(0) as u32;
                    crate::snap_tree::DisplayLineEntry::placeholder(offset, length, 0, 1)
                })
                .collect();
            let _ = tab.display_mut().display_map.sync(dirty_lines.clone(), entries);
            tab.invalidate_render_cache_all();
        } else {
            tab.display_mut().render_cache.invalidate_range(dirty_lines.clone());
        }
    }
    tab.clamp_scroll_anchor(line_height);
    tab.derive_scroll_top(line_height);
    tab.ensure_cursor_visible(line_height);
    tab.cursor_render_state_mut().reset_after_edit();
    if tab.handles_own_rendering() {
        tab.invalidate_cursor_visibility();
    }
}

fn refresh_presentation_after_edit(tab: &mut TabSessionMut<'_>, line_height: f32) {
    let placeholder_entries = placeholder_display_entries(tab.document);
    tab.display_mut().display_map.set_entries(placeholder_entries);
    tab.invalidate_render_cache_all();
    tab.cursor_render_state_mut().click_hint = None;
    tab.cursor_render_state_mut().cursor_blink_instant = std::time::Instant::now();
    if tab.handles_own_rendering() {
        tab.invalidate_cursor_visibility();
    } else {
        tab.ensure_cursor_visible(line_height);
    }
}

fn synchronize_plugin_source(tab: &mut TabSessionMut<'_>) {
    use ui::plugin::PluginMessage;

    if tab.plugin_name() == ui::plugin::PLUGIN_EDITOR {
        return;
    }
    let source = tab.document.full_text();
    let generation = tab.document.generation();
    tab.send_message(PluginMessage::UpdateSource { text: source, generation });
    synchronize_plugin_caret(tab);
}

fn synchronize_plugin_caret(tab: &mut TabSessionMut<'_>) {
    use ui::plugin::{PluginMessage, VisualCaretPosition};
    let byte_offset = tab.document.cursor_offset().to_usize();
    let affinity = tab.document.tb().cursor_affinity();
    let anchor = tab.document.cursor().selection_anchor;
    tab.send_message(PluginMessage::SetSelAnchorByte(anchor));
    tab.send_message(PluginMessage::SetSelCursorByte(anchor.map(|_| byte_offset)));
    tab.send_message(PluginMessage::SetCursorByte(byte_offset));
    tab.send_message(PluginMessage::SetVisualCaret(VisualCaretPosition { byte_offset, affinity }));
}

fn edit_outcome(
    tab_id: TabId,
    previous_content_revision: u64,
    previous_dirty: bool,
    current_content_revision: u64,
    current_dirty: bool,
) -> EditorOutcome {
    let mut notifications = smallvec::SmallVec::new();
    if current_content_revision != previous_content_revision {
        notifications.push(EditorNotification::ContentChanged {
            tab_id,
            content_revision: current_content_revision,
        });
    }
    if current_dirty != previous_dirty {
        notifications.push(EditorNotification::DirtyChanged { tab_id, dirty: current_dirty });
    }
    EditorOutcome { shell_effect: crate::event::ShellEffect::REDRAW, notifications }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;

    use appkit_core::document::DocumentModel;
    use textora_core::buffer::TextBuffer;
    use ui::plugin::{PLUGIN_EDITOR, PLUGIN_MINDMAP, SemanticEditCommand};

    use super::*;
    use crate::editor_plugin::EditorPluginFactory;
    use crate::editor_runtime::OpenDisposition;
    use crate::tab_runtime::TabRuntime;
    use ui::plugin::PluginFactory;

    fn model_session() -> ModelSession {
        let mut registry = ui::plugin::PluginRegistry::new();
        registry.register(Box::new(EditorPluginFactory));
        let routes = ViewRouteTable::new(Vec::new(), &HashSet::from([PLUGIN_EDITOR]))
            .expect("empty test routes should be valid");
        ModelSession::new(registry, routes)
    }

    fn prepared_text(text: &str) -> PreparedTab {
        let mut text_buffer =
            TextBuffer::new(false).expect("model session test requires a writable text buffer");
        text_buffer.write_raw(text.as_bytes());
        PreparedTab::new(
            DocumentModel::new(text_buffer),
            TabRuntime::new(EditorPluginFactory.create()),
        )
    }

    fn prepared_crlf_document(text: &str) -> DocumentModel {
        let mut text_buffer =
            TextBuffer::new(false).expect("model session test requires a writable text buffer");
        text_buffer.write_raw(text.as_bytes());
        text_buffer.set_crlf(true);
        DocumentModel::new(text_buffer)
    }

    struct SourceSyncProbePlugin {
        sources: Rc<RefCell<Vec<(String, u32)>>>,
    }

    struct MindmapThemeProbePlugin;

    impl ui::plugin::ViewPlugin for MindmapThemeProbePlugin {
        fn name(&self) -> &str {
            PLUGIN_MINDMAP
        }

        fn render(
            &mut self,
            _document: &dyn textora_core::document::DocView,
            _bounds: ui::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::DrawList {
            ui::DrawList::new()
        }

        fn query(
            &self,
            query: ui::plugin::PluginQuery,
            _document: &dyn textora_core::document::DocView,
        ) -> ui::plugin::PluginResponse {
            let ui::plugin::PluginQuery::PlanMindmapTheme { theme_id, source_generation } = query
            else {
                return ui::plugin::PluginResponse::None;
            };
            ui::plugin::PluginResponse::EditPlan(ui::plugin::EditPlan::Apply(
                ui::plugin::EditTransaction::replace(
                    source_generation,
                    0..0,
                    format!("theme={theme_id}\n"),
                    0,
                ),
            ))
        }
    }

    impl ui::plugin::ViewPlugin for SourceSyncProbePlugin {
        fn name(&self) -> &str {
            "source-sync-probe"
        }

        fn render(
            &mut self,
            _document: &dyn textora_core::document::DocView,
            _bounds: ui::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::DrawList {
            ui::DrawList::new()
        }

        fn handle_message(
            &mut self,
            message: ui::plugin::PluginMessage,
            _document: &mut dyn textora_core::document::DocViewMut,
        ) -> bool {
            let ui::plugin::PluginMessage::UpdateSource { text, generation } = message else {
                return false;
            };
            self.sources.borrow_mut().push((text, generation));
            true
        }
    }

    struct PlanSyncProbePlugin {
        sources: Rc<RefCell<Vec<(String, u32)>>>,
        plan_source_states: Rc<RefCell<Vec<bool>>>,
    }

    impl PlanSyncProbePlugin {
        fn synced_generation(&self) -> Option<u32> {
            self.sources.borrow().last().map(|(_, generation)| *generation)
        }
    }

    impl ui::plugin::EditPolicy for PlanSyncProbePlugin {
        fn plan_edit(&self, request: &ui::plugin::EditRequest) -> ui::plugin::EditPlan {
            self.plan_source_states
                .borrow_mut()
                .push(self.synced_generation() == Some(request.source_generation));
            ui::plugin::EditPlan::UseDefault
        }
    }

    impl ui::plugin::ViewPlugin for PlanSyncProbePlugin {
        fn name(&self) -> &str {
            "plan-sync-probe"
        }

        fn render(
            &mut self,
            _document: &dyn textora_core::document::DocView,
            _bounds: ui::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::DrawList {
            ui::DrawList::new()
        }

        fn edit_policy(&self) -> &dyn ui::plugin::EditPolicy {
            self
        }

        fn query(
            &self,
            query: ui::plugin::PluginQuery,
            _document: &dyn textora_core::document::DocView,
        ) -> ui::plugin::PluginResponse {
            let ui::plugin::PluginQuery::NeedsSourceUpdate(generation) = query else {
                return ui::plugin::PluginResponse::None;
            };
            ui::plugin::PluginResponse::Bool(self.synced_generation() != Some(generation))
        }

        fn handle_message(
            &mut self,
            message: ui::plugin::PluginMessage,
            _document: &mut dyn textora_core::document::DocViewMut,
        ) -> bool {
            let ui::plugin::PluginMessage::UpdateSource { text, generation } = message else {
                return false;
            };
            self.sources.borrow_mut().push((text, generation));
            true
        }
    }

    #[test]
    fn install_synchronizes_plugin_source_before_the_first_paint() {
        let mut session = model_session();
        let mut text_buffer =
            TextBuffer::new(false).expect("source sync test requires a writable text buffer");
        text_buffer.write_raw(b"#");
        let document = DocumentModel::new(text_buffer);
        let expected_generation = document.generation();
        let sources = Rc::new(RefCell::new(Vec::new()));
        let prepared = PreparedTab::new(
            document,
            TabRuntime::new(Box::new(SourceSyncProbePlugin { sources: Rc::clone(&sources) })),
        );

        let effect = session.install_prepared_tab(prepared, None, OpenDisposition::Persistent);

        assert!(matches!(effect, WorkspaceEffect::Activated(_)));
        assert_eq!(sources.borrow().as_slice(), &[("#".to_owned(), expected_generation)]);
    }

    #[test]
    fn activate_synchronizes_the_target_plugin_source_after_switching_tabs() {
        let mut session = model_session();
        let first_sources = Rc::new(RefCell::new(Vec::new()));
        let first = session.install_prepared_tab(
            PreparedTab::new(
                prepared_text("first").document,
                TabRuntime::new(Box::new(SourceSyncProbePlugin {
                    sources: Rc::clone(&first_sources),
                })),
            ),
            None,
            OpenDisposition::Persistent,
        );
        let first_id = match first {
            WorkspaceEffect::Activated(tab_id) => tab_id,
            _ => panic!("first tab should activate"),
        };

        let second_sources = Rc::new(RefCell::new(Vec::new()));
        let _ = session.install_prepared_tab(
            PreparedTab::new(
                prepared_text("second").document,
                TabRuntime::new(Box::new(SourceSyncProbePlugin {
                    sources: Rc::clone(&second_sources),
                })),
            ),
            None,
            OpenDisposition::Persistent,
        );
        first_sources.borrow_mut().clear();

        let effect = session.activate(first_id).expect("first tab should be addressable");

        assert!(matches!(effect, WorkspaceEffect::Activated(tab_id) if tab_id == first_id));
        assert_eq!(first_sources.borrow().as_slice(), &[("first".to_owned(), 1)]);
    }

    #[test]
    fn install_keeps_model_and_runtime_ids_bijective() {
        let mut session = model_session();
        let first =
            session.install_prepared_tab(prepared_text("first"), None, OpenDisposition::Persistent);
        assert!(matches!(first, WorkspaceEffect::Activated(_)));
        let second = session.install_prepared_tab(
            prepared_text("second"),
            None,
            OpenDisposition::Persistent,
        );
        assert!(matches!(second, WorkspaceEffect::Activated(_)));
        assert_eq!(session.workspace.tab_ids(), session.runtimes.ids());
    }

    #[test]
    fn workspace_snapshot_preserves_model_runtime_bijection() {
        let mut session = model_session();
        let effect = session.install_prepared_tab(
            prepared_text("document"),
            None,
            OpenDisposition::Persistent,
        );
        assert!(matches!(effect, WorkspaceEffect::Activated(_)));

        let active_id =
            session.active_tab_id().expect("installed document should have an active tab");
        let snapshot = session.workspace_snapshot();

        assert_eq!(session.active_tab_id(), Some(active_id));
        assert_eq!(session.workspace.tab_ids(), session.runtimes.ids());
        assert_eq!(
            snapshot.tabs.iter().map(|tab| tab.tab_id).collect::<std::collections::HashSet<_>>(),
            session.workspace.tab_ids()
        );
    }

    #[test]
    fn preview_is_replaced_without_removing_persistent_tabs() {
        let mut session = model_session();
        let persistent = session.install_prepared_tab(
            prepared_text("persistent"),
            None,
            OpenDisposition::Persistent,
        );
        let persistent_id = match persistent {
            WorkspaceEffect::Activated(tab_id) => tab_id,
            _ => panic!("first tab should activate"),
        };
        let preview =
            session.install_prepared_tab(prepared_text("preview"), None, OpenDisposition::Preview);
        let preview_id = session.active_tab_id().expect("preview should activate");
        assert!(matches!(preview, WorkspaceEffect::Activated(_)));

        let replacement = session.install_prepared_tab(
            prepared_text("replacement"),
            None,
            OpenDisposition::Preview,
        );
        let replacement_id = session.active_tab_id().expect("replacement should activate");
        assert!(matches!(replacement, WorkspaceEffect::Closed { .. }));
        assert!(session.document_summary(persistent_id).is_some());
        assert!(session.document_summary(preview_id).is_none());
        assert!(session.document_summary(replacement_id).is_some());
    }

    #[test]
    fn unknown_lifecycle_ids_are_safe_no_ops() {
        let mut session = model_session();
        let mut allocator = appkit_core::workspace::types::TabIdAllocator::new();
        let unknown = allocator.allocate();

        assert!(session.activate(unknown).is_none());
        assert!(session.close_decision(unknown).is_none());
        assert!(session.close(unknown).is_none());
        assert!(session.confirm_close(unknown, CloseConfirmation::Discard).is_none());
    }

    #[test]
    fn cancel_and_saved_confirmation_respect_dirty_state() {
        let mut session = model_session();
        let effect =
            session.install_prepared_tab(prepared_text("dirty"), None, OpenDisposition::Persistent);
        let tab_id = match effect {
            WorkspaceEffect::Activated(tab_id) => tab_id,
            _ => panic!("first tab should activate"),
        };
        let document = session.workspace.entry_by_id(tab_id).expect("installed tab should exist");
        assert!(!document.dirty);

        let discarded = session.confirm_close(tab_id, CloseConfirmation::Cancel);
        assert!(discarded.is_none());
        assert!(session.document_summary(tab_id).is_some());
    }

    #[test]
    fn invalid_resulting_caret_rejects_transaction_before_writing() {
        let mut session = model_session();
        let _ =
            session.install_prepared_tab(prepared_text("abc"), None, OpenDisposition::Persistent);
        let tab_id = session.active_tab_id().expect("installed document is active");
        let before = session.document_text_snapshot(tab_id).expect("document snapshot exists");
        let generation =
            session.tab_session(tab_id).expect("document exists").document.generation();
        let transaction = ui::plugin::EditTransaction::replace(generation, 1..2, "中".into(), 2);

        let outcome = session.apply_active_edit_transaction(transaction, 20.0);

        let after = session.document_text_snapshot(tab_id).expect("document snapshot still exists");
        assert_eq!(after.text, before.text);
        assert_eq!(after.content_revision, before.content_revision);
        assert!(outcome.notifications.is_empty());
    }

    #[test]
    fn unsupported_semantic_command_is_typed_and_does_not_mutate_document() {
        let mut session = model_session();
        let effect =
            session.install_prepared_tab(prepared_text("正文"), None, OpenDisposition::Persistent);
        let tab_id = match effect {
            WorkspaceEffect::Activated(tab_id) => tab_id,
            _ => panic!("first tab should activate"),
        };
        let before = session
            .document_text_snapshot(tab_id)
            .expect("installed tab should expose a text snapshot");

        let (result, outcome) =
            session.execute_semantic_edit(SemanticEditCommand::ToggleBold, 20.0);

        assert_eq!(result, super::super::SemanticEditResult::Unsupported);
        assert!(outcome.notifications.is_empty());
        let after =
            session.document_text_snapshot(tab_id).expect("installed tab should remain available");
        assert_eq!(after.text, before.text);
        assert_eq!(after.content_revision, before.content_revision);
    }

    #[test]
    fn active_mindmap_theme_applies_the_plugin_plan_and_reports_content_change() {
        let mut session = model_session();
        let prepared = PreparedTab::new(
            prepared_text("root").document,
            TabRuntime::new(Box::new(MindmapThemeProbePlugin)),
        );
        let _ = session.install_prepared_tab(prepared, None, OpenDisposition::Persistent);

        let outcome = session.apply_active_mindmap_theme("tide".to_owned(), 20.0);
        let tab_id = session.active_tab_id().expect("mindmap tab should remain active");

        assert_eq!(
            session.document_text_snapshot(tab_id).expect("mindmap snapshot should exist").text,
            "theme=tide\nroot"
        );
        assert!(outcome.notifications.iter().any(|notification| matches!(
            notification,
            EditorNotification::ContentChanged { tab_id: changed_tab_id, .. }
                if *changed_tab_id == tab_id
        )));
    }

    #[test]
    fn default_plan_breaks_paragraph_with_crlf_in_crlf_documents() {
        let document = prepared_crlf_document("alpha\r\nbeta");
        let request = ui::plugin::EditRequest {
            source_generation: document.generation(),
            cursor_byte: 0,
            selection: None,
            intent: ui::plugin::EditIntent::InsertParagraphBreak,
        };

        let plan = default_edit_plan(&request, &document);

        let ui::plugin::EditPlan::ApplyDefault(transaction, _) = plan else {
            panic!("paragraph break should plan a text replacement");
        };
        assert_eq!(transaction.replacements.len(), 1);
        assert_eq!(transaction.replacements[0].text, "\r\n");
    }

    #[test]
    fn default_plan_breaks_paragraph_with_lf_in_lf_documents() {
        let prepared = prepared_text("alpha\nbeta");
        let document = prepared.document;
        let request = ui::plugin::EditRequest {
            source_generation: document.generation(),
            cursor_byte: 0,
            selection: None,
            intent: ui::plugin::EditIntent::InsertParagraphBreak,
        };

        let plan = default_edit_plan(&request, &document);

        let ui::plugin::EditPlan::ApplyDefault(transaction, _) = plan else {
            panic!("paragraph break should plan a text replacement");
        };
        assert_eq!(transaction.replacements.len(), 1);
        assert_eq!(transaction.replacements[0].text, "\n");
    }

    #[test]
    fn default_line_break_falls_back_to_the_native_document_newline() {
        let document = prepared_crlf_document("alpha\r\nbeta");
        let request = ui::plugin::EditRequest {
            source_generation: document.generation(),
            cursor_byte: 0,
            selection: None,
            intent: ui::plugin::EditIntent::InsertLineBreak,
        };

        let ui::plugin::EditPlan::ApplyDefault(transaction, _) =
            default_edit_plan(&request, &document)
        else {
            panic!("line break should use the default newline outside Markdown WYSIWYG");
        };
        assert_eq!(transaction.replacements[0].text, "\r\n");
    }

    fn default_plan_history_kind(
        document: &DocumentModel,
        intent: ui::plugin::EditIntent,
    ) -> textora_core::buffer::EditHistoryKind {
        let request = ui::plugin::EditRequest {
            source_generation: document.generation(),
            cursor_byte: document.cursor_offset().to_usize(),
            selection: None,
            intent,
        };
        let ui::plugin::EditPlan::ApplyDefault(_, history) = default_edit_plan(&request, document)
        else {
            panic!("default plan should apply a transaction with a history kind");
        };
        history
    }

    #[test]
    fn default_plan_marks_paragraph_break_and_indent_as_standalone_history() {
        let document = prepared_text("").document;

        assert_eq!(
            default_plan_history_kind(&document, ui::plugin::EditIntent::InsertParagraphBreak),
            textora_core::buffer::EditHistoryKind::Standalone,
            "a paragraph break must never join a typing run"
        );
        assert_eq!(
            default_plan_history_kind(&document, ui::plugin::EditIntent::Indent),
            textora_core::buffer::EditHistoryKind::Standalone,
            "an indent must never join a typing run"
        );
        assert_eq!(
            default_plan_history_kind(
                &document,
                ui::plugin::EditIntent::InsertText("x".to_owned())
            ),
            textora_core::buffer::EditHistoryKind::Insert,
            "plain typing keeps coalescing"
        );
    }

    #[test]
    fn default_paragraph_break_and_typing_undo_separately() {
        let mut document = prepared_text("").document;

        let break_request = ui::plugin::EditRequest {
            source_generation: document.generation(),
            cursor_byte: document.cursor_offset().to_usize(),
            selection: None,
            intent: ui::plugin::EditIntent::InsertParagraphBreak,
        };
        let break_plan = default_edit_plan(&break_request, &document);
        let outcome =
            execute_edit_plan(break_plan, &mut document).expect("paragraph break should apply");
        assert!(outcome.edit_outcome.executed);
        let type_request = ui::plugin::EditRequest {
            source_generation: document.generation(),
            cursor_byte: document.cursor_offset().to_usize(),
            selection: None,
            intent: ui::plugin::EditIntent::InsertText("x".to_owned()),
        };
        let type_plan = default_edit_plan(&type_request, &document);
        let outcome = execute_edit_plan(type_plan, &mut document).expect("typing should apply");
        assert!(outcome.edit_outcome.executed);

        assert_eq!(document.full_text(), "\nx");
        document.undo();
        assert_eq!(document.full_text(), "\n", "first undo must only remove the typed text");
        document.undo();
        assert_eq!(document.full_text(), "", "second undo must only remove the break");
    }

    fn active_document_text(session: &ModelSession, tab_id: TabId) -> String {
        session
            .document_text_snapshot(tab_id)
            .expect("installed tab should expose a text snapshot")
            .text
    }

    #[test]
    fn typing_without_navigation_coalesces_into_one_undo_entry() {
        let mut session = model_session();
        let effect =
            session.install_prepared_tab(prepared_text(""), None, OpenDisposition::Persistent);
        let tab_id = match effect {
            WorkspaceEffect::Activated(tab_id) => tab_id,
            _ => panic!("coalescing tab should activate"),
        };

        session.edit_active_document(ui::plugin::EditIntent::InsertText("a".to_owned()), 20.0);
        session.edit_active_document(ui::plugin::EditIntent::InsertText("b".to_owned()), 20.0);

        assert_eq!(active_document_text(&session, tab_id), "ab");
        session.undo_or_redo_active_document(false, 20.0);
        assert_eq!(
            active_document_text(&session, tab_id),
            "",
            "one undo must revert the whole typing run"
        );
    }

    #[test]
    fn keyboard_navigation_between_typing_splits_undo_entries() {
        let mut session = model_session();
        let effect =
            session.install_prepared_tab(prepared_text(""), None, OpenDisposition::Persistent);
        let tab_id = match effect {
            WorkspaceEffect::Activated(tab_id) => tab_id,
            _ => panic!("navigation tab should activate"),
        };

        session.edit_active_document(ui::plugin::EditIntent::InsertText("a".to_owned()), 20.0);
        // The caret leaves and returns to the exact byte where typing ended.
        session.navigate_active_document(ui::KeyCode::Left, ui::core::Modifiers::NONE, 20.0);
        session.navigate_active_document(ui::KeyCode::Right, ui::core::Modifiers::NONE, 20.0);
        session.edit_active_document(ui::plugin::EditIntent::InsertText("b".to_owned()), 20.0);

        assert_eq!(active_document_text(&session, tab_id), "ab");
        session.undo_or_redo_active_document(false, 20.0);
        assert_eq!(
            active_document_text(&session, tab_id),
            "a",
            "first undo must only remove the text typed after navigating"
        );
        session.undo_or_redo_active_document(false, 20.0);
        assert_eq!(active_document_text(&session, tab_id), "");
    }

    #[test]
    fn undo_synchronizes_plugin_source_without_waiting_for_a_paint() {
        let mut session = model_session();
        let sources = Rc::new(RefCell::new(Vec::new()));
        let prepared = PreparedTab::new(
            prepared_text("ab").document,
            TabRuntime::new(Box::new(SourceSyncProbePlugin { sources: Rc::clone(&sources) })),
        );
        let effect = session.install_prepared_tab(prepared, None, OpenDisposition::Persistent);
        let tab_id = match effect {
            WorkspaceEffect::Activated(tab_id) => tab_id,
            _ => panic!("undo sync tab should activate"),
        };
        let outcome =
            session.edit_active_document(ui::plugin::EditIntent::InsertText("c".to_owned()), 20.0);
        assert!(
            outcome.notifications.iter().any(|notification| matches!(
                notification,
                EditorNotification::ContentChanged { .. }
            ))
        );
        sources.borrow_mut().clear();

        let _ = session.undo_or_redo_active_document(false, 20.0);

        let generation = session
            .workspace
            .entry_by_id(tab_id)
            .expect("undo sync tab should remain installed")
            .generation();
        assert_eq!(sources.borrow().as_slice(), &[("ab".to_owned(), generation)]);
    }

    #[test]
    fn edit_planning_synchronizes_stale_plugin_source_before_planning() {
        let mut session = model_session();
        let sources = Rc::new(RefCell::new(Vec::new()));
        let plan_source_states = Rc::new(RefCell::new(Vec::new()));
        let prepared = PreparedTab::new(
            prepared_text("ab").document,
            TabRuntime::new(Box::new(PlanSyncProbePlugin {
                sources: Rc::clone(&sources),
                plan_source_states: Rc::clone(&plan_source_states),
            })),
        );
        let effect = session.install_prepared_tab(prepared, None, OpenDisposition::Persistent);
        let tab_id = match effect {
            WorkspaceEffect::Activated(tab_id) => tab_id,
            _ => panic!("plan sync tab should activate"),
        };
        // Simulate an out-of-band mutation (e.g. undo) that left the plugin
        // source cache stale relative to the document generation.
        {
            let index =
                session.workspace.index_of(tab_id).expect("plan sync tab should be indexed");
            let document =
                session.workspace.entry_mut(index).expect("plan sync tab should remain installed");
            document.tb.replace_range(0..0, b"z");
            document.line_index = appkit_core::line_index::LineIndex::rebuild_from(&document.tb);
            document.mark_content_changed();
        }

        let outcome =
            session.edit_active_document(ui::plugin::EditIntent::InsertText("y".to_owned()), 20.0);

        assert!(
            outcome.notifications.iter().any(|notification| matches!(
                notification,
                EditorNotification::ContentChanged { .. }
            ))
        );
        assert_eq!(plan_source_states.borrow().as_slice(), &[true]);
        let synced_texts: Vec<String> =
            sources.borrow().iter().map(|(text, _)| text.clone()).collect();
        assert_eq!(synced_texts, ["ab", "zab", "zyab"]);
    }
}
