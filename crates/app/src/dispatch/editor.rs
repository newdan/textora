use crate::app::App;
use crate::app::reset_cursor_after_edit;
use crate::app_effect::AppEffect;
use crate::commands::EditOutcome;
use crate::input::EditCommand;
use crate::ui_shell::KeyboardFocusTarget;
use appkit_shell::editor_runtime::{EditorNotification, EditorOutcome};
use winit::event_loop::ActiveEventLoop;

/// Caret-moving commands that must split any ongoing undo coalescing run.
/// Includes selection extension and jumps (SelectAll, navigation history):
/// anything that repositions the caret without editing text. Editing commands
/// never reach this predicate: they are handled earlier by the transactional
/// edit path.
fn is_cursor_navigation_command(cmd: &EditCommand) -> bool {
    matches!(
        cmd,
        EditCommand::MoveLeft
            | EditCommand::MoveRight
            | EditCommand::MoveUp
            | EditCommand::MoveDown
            | EditCommand::MoveWordLeft
            | EditCommand::MoveWordRight
            | EditCommand::MoveToLineStart
            | EditCommand::MoveToLineEnd
            | EditCommand::MoveToDocStart
            | EditCommand::MoveToDocEnd
            | EditCommand::PageUp
            | EditCommand::PageDown
            | EditCommand::ExtendLeft
            | EditCommand::ExtendRight
            | EditCommand::ExtendUp
            | EditCommand::ExtendDown
            | EditCommand::ExtendWordLeft
            | EditCommand::ExtendWordRight
            | EditCommand::ExtendToLineStart
            | EditCommand::ExtendToLineEnd
            | EditCommand::ExtendToDocStart
            | EditCommand::ExtendToDocEnd
            | EditCommand::SelectAll
            | EditCommand::NavigateBack
            | EditCommand::NavigateForward
    )
}

/// Visual-navigation commands are routed to the WYSIWYG plugin so it can
/// resolve the target from its own layout. Text-editing commands never reach
/// this predicate: they are handled earlier by the transactional edit path.
fn is_wysiwyg_navigation_command(cmd: &EditCommand) -> bool {
    matches!(
        cmd,
        EditCommand::MoveLeft
            | EditCommand::MoveRight
            | EditCommand::MoveUp
            | EditCommand::MoveDown
            | EditCommand::MoveToLineStart
            | EditCommand::MoveToLineEnd
            | EditCommand::MoveToDocStart
            | EditCommand::MoveToDocEnd
            | EditCommand::ExtendLeft
            | EditCommand::ExtendRight
            | EditCommand::ExtendUp
            | EditCommand::ExtendDown
            | EditCommand::ExtendToLineStart
            | EditCommand::ExtendToLineEnd
            | EditCommand::ExtendToDocStart
            | EditCommand::ExtendToDocEnd
            | EditCommand::PageUp
            | EditCommand::PageDown
    )
}

fn search_panel_receives_edit_commands(
    search_panel_visible: bool,
    keyboard_focus: KeyboardFocusTarget,
) -> bool {
    search_panel_visible
        && keyboard_focus == KeyboardFocusTarget::Widget(ui::core::widget::ids::SEARCH_BAR)
}

fn edit_requires_reshape(cmd: &EditCommand, outcome: &EditOutcome) -> bool {
    if !outcome.executed {
        return false;
    }

    if outcome.new_line_count != outcome.old_line_count {
        return true;
    }

    if matches!(
        cmd,
        EditCommand::Paste
            | EditCommand::PastePlainText
            | EditCommand::Undo
            | EditCommand::Redo
            | EditCommand::Cut
    ) {
        return true;
    }

    outcome.dirty_lines.as_ref().is_some_and(|range| range.len() > 1)
}

fn editor_edit_outcome(
    tab_id: appkit_core::workspace::types::TabId,
    previous_content_revision: u64,
    previous_dirty: bool,
    current_content_revision: u64,
    current_dirty: bool,
    shell_effect: AppEffect,
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
    EditorOutcome { shell_effect, notifications }
}

fn apply_editor_dirty_notifications(
    app: &App,
    notifications: &[EditorNotification],
    fallback_dirty: bool,
) {
    let dirty = notifications
        .iter()
        .find_map(|notification| match notification {
            EditorNotification::DirtyChanged { dirty, .. } => Some(*dirty),
            _ => None,
        })
        .unwrap_or(fallback_dirty);
    app.update_document_edited(dirty);
}

impl App {
    pub(crate) fn dispatch_semantic_edit(
        &mut self,
        command: ui::plugin::SemanticEditCommand,
    ) -> AppEffect {
        self.editor_runtime.set_preferred_x(None);
        self.sync_plugin_state();

        let (_result, outcome) = self.editor_runtime.execute_semantic_edit(command);
        let current_dirty = self.active_tab_session().is_some_and(|tab| tab.document.dirty);
        apply_editor_dirty_notifications(self, &outcome.notifications, current_dirty);
        self.sync_plugin_state();

        outcome.shell_effect
    }

    fn dispatch_keyboard_tab_switch(&mut self, command: &EditCommand) -> AppEffect {
        let index = match command {
            EditCommand::NextTab if !self.editor_is_empty() => {
                (self.active_editor_index().unwrap_or(0) + 1) % self.editor_tab_count()
            }
            EditCommand::PrevTab if !self.editor_is_empty() => {
                if self.active_editor_index().unwrap_or(0) == 0 {
                    self.editor_tab_count() - 1
                } else {
                    self.active_editor_index().unwrap_or(0) - 1
                }
            }
            EditCommand::SwitchTab(index) => *index,
            _ => return AppEffect::NONE,
        };
        if let Some(id) = self.editor_tab_id_at(index) {
            self.dispatch_tab_switch(id)
        } else {
            AppEffect::NONE
        }
    }

    /// Splits the active document's undo coalescing run when `cmd` is a
    /// user-driven caret move, so typing after navigation starts a fresh undo
    /// entry even if the caret returns to the byte where the last edit ended.
    pub(crate) fn break_edit_merge_for_navigation(&mut self, cmd: &EditCommand) {
        if !is_cursor_navigation_command(cmd) {
            return;
        }
        if let Some(tab) = self.active_tab_session_mut() {
            tab.document.break_edit_merge();
        }
    }

    pub(crate) fn dispatch_edit_command(
        &mut self,
        cmd: EditCommand,
        event_loop: &ActiveEventLoop,
    ) -> AppEffect {
        let mut effect = AppEffect::NONE;

        // ── Markdown preview mode: block editing commands ──
        if !self.active_allows_editing() {
            match cmd {
                EditCommand::ToggleView
                | EditCommand::Escape
                | EditCommand::PageUp
                | EditCommand::PageDown
                | EditCommand::MoveUp
                | EditCommand::MoveDown
                | EditCommand::MoveLeft
                | EditCommand::MoveRight
                | EditCommand::MoveToLineStart
                | EditCommand::MoveToLineEnd
                | EditCommand::MoveToDocStart
                | EditCommand::MoveToDocEnd
                | EditCommand::Save
                | EditCommand::SaveAs
                | EditCommand::CloseTab
                | EditCommand::NextTab
                | EditCommand::PrevTab
                | EditCommand::NavigateBack
                | EditCommand::NavigateForward
                | EditCommand::ToggleSidebarPin
                | EditCommand::ToggleToc
                | EditCommand::Find
                | EditCommand::FindNext
                | EditCommand::FindPrev
                | EditCommand::Copy
                | EditCommand::Cut
                | EditCommand::SelectAll
                | EditCommand::ExtendLeft
                | EditCommand::ExtendRight
                | EditCommand::ExtendUp
                | EditCommand::ExtendDown
                | EditCommand::ExtendToLineStart
                | EditCommand::ExtendToLineEnd
                | EditCommand::ExtendToDocStart
                | EditCommand::ExtendToDocEnd
                | EditCommand::NextChapter
                | EditCommand::PrevChapter => {}
                _ => return effect,
            }
        }

        if !self.active_allows_editing() {
            match cmd {
                EditCommand::Copy | EditCommand::Cut => {
                    if let Some(tab) = self.active_tab_session()
                        && let text = tab.selected_text()
                        && !text.is_empty()
                    {
                        crate::clipboard::copy_to_clipboard(&text);
                    }
                    return effect;
                }
                EditCommand::SelectAll => {
                    if let Some(mut tab) = self.active_tab_session_mut() {
                        tab.send_message(ui::plugin::PluginMessage::SelectAll);
                        effect = effect.merge(AppEffect::REDRAW);
                    }
                    return effect;
                }
                EditCommand::ExtendLeft => {
                    if let Some(mut tab) = self.active_tab_session_mut() {
                        let cursor = tab.selection_cursor();
                        if let Some((li, cp)) = cursor {
                            let flat_lines = tab.flat_lines();
                            let mut new_li = li;
                            let mut new_cp = cp;
                            if cp > 0 {
                                new_cp -= 1;
                            } else if li > 0 {
                                new_li -= 1;
                                new_cp = flat_lines.get(new_li).map_or(0, |fl| fl.grapheme_count);
                            }
                            tab.send_message(ui::plugin::PluginMessage::SetSelCursor(Some((
                                new_li, new_cp,
                            ))));
                        }
                        effect = effect.merge(AppEffect::REDRAW);
                    }
                    return effect;
                }
                EditCommand::ExtendRight => {
                    if let Some(mut tab) = self.active_tab_session_mut() {
                        if let Some((li, cp)) = tab.selection_cursor() {
                            let flat_lines = tab.flat_lines();
                            let line_len = flat_lines.get(li).map_or(0, |fl| fl.grapheme_count);
                            let total_lines = flat_lines.len();
                            let mut new_li = li;
                            let mut new_cp = cp;
                            if cp < line_len {
                                new_cp += 1;
                            } else if li + 1 < total_lines {
                                new_li += 1;
                                new_cp = 0;
                            }
                            tab.send_message(ui::plugin::PluginMessage::SetSelCursor(Some((
                                new_li, new_cp,
                            ))));
                        }
                        effect = effect.merge(AppEffect::REDRAW);
                    }
                    return effect;
                }
                EditCommand::ExtendToLineStart => {
                    if let Some(mut tab) = self.active_tab_session_mut() {
                        if let Some((li, _cp)) = tab.selection_cursor() {
                            tab.send_message(ui::plugin::PluginMessage::SetSelCursor(Some((
                                li, 0,
                            ))));
                        }
                        effect = effect.merge(AppEffect::REDRAW);
                    }
                    return effect;
                }
                EditCommand::ExtendToLineEnd => {
                    if let Some(mut tab) = self.active_tab_session_mut() {
                        if let Some((li, _cp)) = tab.selection_cursor() {
                            let flat_lines = tab.flat_lines();
                            let grapheme_count =
                                flat_lines.get(li).map_or(0, |fl| fl.grapheme_count);
                            tab.send_message(ui::plugin::PluginMessage::SetSelCursor(Some((
                                li,
                                grapheme_count,
                            ))));
                        }
                        effect = effect.merge(AppEffect::REDRAW);
                    }
                    return effect;
                }
                EditCommand::ExtendToDocStart => {
                    if let Some(mut tab) = self.active_tab_session_mut() {
                        tab.send_message(ui::plugin::PluginMessage::SetSelCursor(Some((0, 0))));
                        effect = effect.merge(AppEffect::REDRAW);
                    }
                    return effect;
                }
                EditCommand::ExtendToDocEnd => {
                    if let Some(mut tab) = self.active_tab_session_mut() {
                        let flat_lines = tab.flat_lines();
                        let total = flat_lines.len();
                        if total > 0 {
                            let last_g = flat_lines.last().map_or(0, |fl| fl.grapheme_count);
                            tab.send_message(ui::plugin::PluginMessage::SetSelCursor(Some((
                                total - 1,
                                last_g,
                            ))));
                        }
                        effect = effect.merge(AppEffect::REDRAW);
                    }
                    return effect;
                }
                _ => {}
            }
        }

        if self.active_allows_editing() {
            self.sync_plugin_state();

            if let Some(intent) = crate::edit_transaction::edit_intent_for_command(&cmd) {
                return self.dispatch_transactional_edit(intent, Some(event_loop));
            }

            // User-driven caret movement splits the undo coalescing run before
            // the move is routed to source mode, WYSIWYG mode, or paging.
            self.break_edit_merge_for_navigation(&cmd);

            if self.active_handles_own_rendering() && is_wysiwyg_navigation_command(&cmd) {
                return self.dispatch_wysiwyg_navigation(&cmd);
            }
        }

        if matches!(self.settings.view_mode, ui::view_mode::ViewMode::Sidebar) {
            match &cmd {
                EditCommand::Escape => {
                    if let Some(mut tab) = self.active_tab_session_mut()
                        && tab.search_state().panel_visible
                    {
                        tab.search_state_mut().dismiss_or_clear();
                        self.ui_shell.focus_editor();
                        effect = effect.merge(AppEffect::REDRAW);
                        return effect;
                    }
                    let action = {
                        let shell = &mut self.ui_shell;
                        shell.sidebar_on_key(ui::sidebar::SidebarKey::Escape)
                    };
                    if let Some(action) = action {
                        effect = effect.merge(self.handle_sidebar_key_action(action));
                        return effect;
                    }
                    self.quit_app(event_loop);
                    return effect;
                }
                EditCommand::ToggleSidebarPin => {
                    let action = {
                        let shell = &mut self.ui_shell;
                        shell.sidebar_on_key(ui::sidebar::SidebarKey::TogglePin)
                    };
                    if let Some(action) = action {
                        effect = effect.merge(self.handle_sidebar_key_action(action));
                    }
                    return effect;
                }
                _ => {}
            }
        }

        if cmd == EditCommand::Escape {
            if let Some(mut tab) = self.active_tab_session_mut()
                && tab.search_state().panel_visible
            {
                tab.search_state_mut().dismiss_or_clear();
                self.ui_shell.focus_editor();
                effect = effect.merge(AppEffect::REDRAW);
                return effect;
            }
            self.quit_app(event_loop);
            return effect;
        }

        match &cmd {
            EditCommand::MoveUp => {
                effect = effect.merge(self.move_cursor_visual(-1));
                return effect;
            }
            EditCommand::MoveDown => {
                effect = effect.merge(self.move_cursor_visual(1));
                return effect;
            }
            EditCommand::ExtendUp => {
                effect = effect.merge(self.extend_selection_visual(-1));
                return effect;
            }
            EditCommand::ExtendDown => {
                effect = effect.merge(self.extend_selection_visual(1));
                return effect;
            }
            EditCommand::ToggleSidebarPin => return effect,
            EditCommand::ToggleView => {
                self.switch_active_plugin();
                let h = self.screen_height();
                let visible_rows = self.visible_rows(h);
                let viewport_height = self.visible_height_lines(h);
                if let Some(mut tab) = self.active_tab_session_mut() {
                    tab.resize_presentation(visible_rows, viewport_height);
                }
                if let Some(active_index) = self.active_editor_index() {
                    self.init_display_map(active_index);
                }
                if let Some(mut tab) = self.active_tab_session_mut() {
                    tab.clear_advance_cache();
                }
                self.editor_runtime.clear_frame_cluster_pool();
                effect = effect.merge(AppEffect::REDRAW);
                return effect;
            }
            EditCommand::ToggleToc => {
                let in_preview = self.active_handles_own_rendering();
                if in_preview {
                    if let Some(mut tab) = self.active_tab_session_mut() {
                        tab.toggle_toc_visible();
                    }
                    effect = effect.merge(AppEffect::REDRAW);
                    return effect;
                }
                return effect;
            }
            EditCommand::NextChapter => {
                if let Some(mut tab) = self.active_tab_session_mut() {
                    tab.send_message(ui::plugin::PluginMessage::ScrollToNextChapter);
                    effect = effect.merge(AppEffect::REDRAW);
                }
                return effect;
            }
            EditCommand::PrevChapter => {
                if let Some(mut tab) = self.active_tab_session_mut() {
                    tab.send_message(ui::plugin::PluginMessage::ScrollToPrevChapter);
                    effect = effect.merge(AppEffect::REDRAW);
                }
                return effect;
            }
            EditCommand::PageUp => {
                effect = effect.merge(self.page_up());
                return effect;
            }
            EditCommand::PageDown => {
                effect = effect.merge(self.page_down());
                return effect;
            }
            EditCommand::Save => {
                effect = effect.merge(self.save_active_entry(false));
                return effect;
            }
            EditCommand::SaveAs => {
                effect = effect.merge(self.save_active_entry(true));
                return effect;
            }
            EditCommand::OpenFile => {
                self.open_file_dialog();
                return effect;
            }
            EditCommand::NewTab => {
                self.new_untitled_doc();
                return effect;
            }
            EditCommand::CloseTab => {
                if let Some(id) = self.active_tab_id() {
                    self.try_close_entry_with_prompt(id);
                }
                return effect;
            }
            EditCommand::ReopenTab => return effect,
            EditCommand::NextTab | EditCommand::PrevTab | EditCommand::SwitchTab(_) => {
                return self.dispatch_keyboard_tab_switch(&cmd);
            }
            EditCommand::NavigateBack => {
                let ws_effect = self.navigate_editor_back();
                self.handle_nav_effect(ws_effect);
                return effect;
            }
            EditCommand::NavigateForward => {
                let ws_effect = self.navigate_editor_forward();
                self.handle_nav_effect(ws_effect);
                return effect;
            }
            _ => {}
        }

        match &cmd {
            EditCommand::Find => {
                if let Some(mut tab) = self.active_tab_session_mut() {
                    let search_state = tab.search_state_mut();
                    search_state.panel_visible = !search_state.panel_visible;
                    if !search_state.panel_visible {
                        search_state.clear();
                        self.ui_shell.focus_editor();
                    } else {
                        self.ui_shell.focus_widget(ui::core::widget::ids::SEARCH_BAR);
                    }
                    effect = effect.merge(AppEffect::REDRAW);
                }
                return effect;
            }
            EditCommand::FindReplace => {
                if let Some(mut tab) = self.active_tab_session_mut() {
                    let search_state = tab.search_state_mut();
                    search_state.panel_visible = true;
                    if !search_state.replace_mode {
                        search_state.toggle_replace_mode();
                    }
                    search_state.focus_replace = true;
                    self.ui_shell.focus_widget(ui::core::widget::ids::SEARCH_BAR);
                    effect = effect.merge(AppEffect::REDRAW);
                }
                return effect;
            }
            EditCommand::FindNext => {
                if let Some(mut tab) = self.active_tab_session_mut()
                    && tab.search_state().is_active()
                {
                    tab.search_state_mut().next_match();
                    self.scroll_to_active_match();
                    effect = effect.merge(AppEffect::REDRAW);
                }
                return effect;
            }
            EditCommand::FindPrev => {
                if let Some(mut tab) = self.active_tab_session_mut()
                    && tab.search_state().is_active()
                {
                    tab.search_state_mut().prev_match();
                    self.scroll_to_active_match();
                    effect = effect.merge(AppEffect::REDRAW);
                }
                return effect;
            }
            EditCommand::InsertChar(ch) => {
                let shell = &self.ui_shell;
                let keyboard_focus = shell.keyboard_focus();
                if let Some(mut tab) = self.active_tab_session_mut()
                    && search_panel_receives_edit_commands(
                        tab.search_state().panel_visible,
                        keyboard_focus,
                    )
                {
                    let search_state = tab.search_state_mut();
                    search_state.query.push_str(ch);
                    search_state.set_cursor_byte_pos(search_state.query.len());
                    tab.cursor_render_state_mut().cursor_blink_instant = std::time::Instant::now();
                    self.perform_search_for_active_doc();
                    effect = effect.merge(AppEffect::REDRAW);
                    return effect;
                }
            }
            EditCommand::Backspace => {
                let shell = &self.ui_shell;
                let keyboard_focus = shell.keyboard_focus();
                if let Some(mut tab) = self.active_tab_session_mut()
                    && search_panel_receives_edit_commands(
                        tab.search_state().panel_visible,
                        keyboard_focus,
                    )
                {
                    let search_state = tab.search_state_mut();
                    search_state.query.pop();
                    search_state.set_cursor_byte_pos(search_state.query.len());
                    tab.cursor_render_state_mut().cursor_blink_instant = std::time::Instant::now();
                    self.perform_search_for_active_doc();
                    effect = effect.merge(AppEffect::REDRAW);
                    return effect;
                }
            }
            EditCommand::InsertNewline => {
                let shell = &self.ui_shell;
                let keyboard_focus = shell.keyboard_focus();
                if let Some(mut tab) = self.active_tab_session_mut()
                    && search_panel_receives_edit_commands(
                        tab.search_state().panel_visible,
                        keyboard_focus,
                    )
                {
                    tab.search_state_mut().panel_visible = false;
                    effect = effect.merge(AppEffect::REDRAW);
                    return effect;
                }
            }
            EditCommand::Tab => {}
            _ => {}
        }

        let ws_effect = self.upgrade_active_editor_preview();
        self.handle_nav_effect(ws_effect);

        let lh = self.ui_metrics().line_height;
        let Some(tab_id) = self.active_tab_id() else {
            return effect;
        };
        let previous_summary = self.editor_runtime.document_summary(tab_id);
        let previous_content_revision =
            previous_summary.as_ref().map_or(0, |summary| summary.content_revision);
        let previous_dirty = previous_summary.as_ref().is_some_and(|summary| summary.dirty);
        let Some(mut tab) = self.active_tab_session_mut() else {
            return effect;
        };
        let ac = tab.take_advance_cache();
        let mut presentation = tab.take_presentation();
        let page_step_rows = presentation.display.viewport.visible_rows.saturating_sub(1).max(1);
        let outcome = crate::commands::execute_edit_command_v2_with_presentation(
            &cmd,
            tab.document,
            &ac,
            &mut presentation.cursor_render_state,
            page_step_rows,
        );
        tab.restore_presentation(presentation);
        tab.restore_advance_cache(ac);
        let mut current_content_revision = previous_content_revision;
        let mut current_dirty = previous_dirty;
        if outcome.executed {
            current_content_revision = tab.document.content_revision();
            current_dirty = tab.document.dirty;
            let requires_reshape = edit_requires_reshape(&cmd, &outcome);
            if outcome.new_line_count != outcome.old_line_count {
                let is_insertion = outcome.new_line_count > outcome.old_line_count;
                let _edit_line = if is_insertion {
                    tab.document.cursor_line()
                } else {
                    tab.document.cursor_line().min(outcome.old_line_count.saturating_sub(1))
                };
                if let Some(ref dirty) = outcome.dirty_lines {
                    let new_entries_len = (dirty.len() as isize
                        + (outcome.new_line_count as isize - outcome.old_line_count as isize))
                        .max(0) as usize;
                    let mut new_entries = Vec::with_capacity(new_entries_len);
                    for dl in dirty.start..dirty.start + new_entries_len {
                        let offset = tab.document.line_byte_offset(dl).unwrap_or(0);
                        let length = tab.document.line_byte_length(dl).unwrap_or(0) as u32;
                        new_entries.push(crate::snap_tree::DisplayLineEntry::placeholder(
                            offset, length, 0, 1,
                        ));
                    }
                    let _ = tab.display_mut().display_map.sync(dirty.clone(), new_entries);
                }
            }
            if outcome.invalidates_all_render_cache() {
                tab.invalidate_render_cache_all();
            } else if let Some(cache_invalidation_range) = outcome.render_cache_invalidation_range()
            {
                tab.display_mut().render_cache.invalidate_range(cache_invalidation_range);
            }
            if let Some(range) = outcome.dirty_lines {
                for doc_line in range.clone() {
                    if doc_line < tab.display().display_map.line_count() {}
                }
            }
            tab.clamp_scroll_anchor(lh);
            tab.derive_scroll_top(lh);
            tab.ensure_cursor_visible(lh);
            reset_cursor_after_edit(tab.cursor_render_state_mut());
            if requires_reshape {
                effect = effect.merge(AppEffect::RESHAPE);
            }
            effect = effect.merge(AppEffect::REDRAW);
        }
        let editor_outcome = editor_edit_outcome(
            tab_id,
            previous_content_revision,
            previous_dirty,
            current_content_revision,
            current_dirty,
            effect,
        );
        apply_editor_dirty_notifications(self, &editor_outcome.notifications, current_dirty);

        let allows_editing = self.active_allows_editing();
        if allows_editing {
            self.sync_plugin_state();
        }

        editor_outcome.shell_effect
    }

    pub(crate) fn dispatch_transactional_edit(
        &mut self,
        intent: ui::plugin::EditIntent,
        _event_loop: Option<&ActiveEventLoop>,
    ) -> AppEffect {
        self.editor_runtime.set_preferred_x(None);
        let mut effect = AppEffect::NONE;
        let Some(tab_id) = self.active_tab_id() else {
            return effect;
        };
        let previous_summary = self.editor_runtime.document_summary(tab_id);
        let previous_content_revision =
            previous_summary.as_ref().map_or(0, |summary| summary.content_revision);
        let previous_dirty = previous_summary.as_ref().is_some_and(|summary| summary.dirty);

        self.sync_plugin_state();
        let lh = self.ui_metrics().line_height;

        let resolved_plan = {
            let Some(tab) = self.active_tab_session() else {
                return effect;
            };
            let request = crate::edit_transaction::build_edit_request(tab.document, intent);
            let plan = tab.plan_edit_request(&request);
            if plan == ui::plugin::EditPlan::UseDefault {
                crate::edit_transaction::default_edit_plan(&request, tab.document)
            } else {
                plan
            }
        };

        let Some(mut tab) = self.active_tab_session_mut() else {
            return effect;
        };
        let outcome_result =
            crate::edit_transaction::execute_edit_plan(resolved_plan, tab.document);

        let mut current_content_revision = previous_content_revision;
        let mut current_dirty = previous_dirty;
        if let Ok(outcome) = outcome_result {
            let text_changed = outcome.edit_outcome.executed;
            current_content_revision = outcome.content_revision;
            current_dirty = outcome.dirty;
            let requires_reshape = text_changed
                && (outcome.edit_outcome.new_line_count != outcome.edit_outcome.old_line_count
                    || outcome
                        .edit_outcome
                        .dirty_lines
                        .as_ref()
                        .is_some_and(|range| range.len() > 1));

            if text_changed
                && outcome.edit_outcome.new_line_count != outcome.edit_outcome.old_line_count
                && let Some(ref dirty) = outcome.edit_outcome.dirty_lines
            {
                let new_entries_len = (dirty.len() as isize
                    + (outcome.edit_outcome.new_line_count as isize
                        - outcome.edit_outcome.old_line_count as isize))
                    .max(0) as usize;
                let mut new_entries = Vec::with_capacity(new_entries_len);
                for dl in dirty.start..dirty.start + new_entries_len {
                    let offset = tab.document.line_byte_offset(dl).unwrap_or(0);
                    let length = tab.document.line_byte_length(dl).unwrap_or(0) as u32;
                    new_entries.push(crate::snap_tree::DisplayLineEntry::placeholder(
                        offset, length, 0, 1,
                    ));
                }
                let _ = tab.display_mut().display_map.sync(dirty.clone(), new_entries);
            }

            if text_changed {
                if outcome.edit_outcome.invalidates_all_render_cache() {
                    tab.invalidate_render_cache_all();
                } else if let Some(cache_range) =
                    outcome.edit_outcome.render_cache_invalidation_range()
                {
                    tab.display_mut().render_cache.invalidate_range(cache_range);
                }
            }

            if text_changed || outcome.cursor_moved {
                tab.clamp_scroll_anchor(lh);
                tab.derive_scroll_top(lh);
                tab.ensure_cursor_visible(lh);

                reset_cursor_after_edit(tab.cursor_render_state_mut());
            }

            if requires_reshape {
                effect = effect.merge(AppEffect::RESHAPE);
            }
            if text_changed {
                effect = effect.merge(AppEffect::REDRAW);
            }
        }

        self.sync_plugin_state();

        let editor_outcome = editor_edit_outcome(
            tab_id,
            previous_content_revision,
            previous_dirty,
            current_content_revision,
            current_dirty,
            effect,
        );
        apply_editor_dirty_notifications(self, &editor_outcome.notifications, current_dirty);

        editor_outcome.shell_effect.merge(AppEffect::REDRAW)
    }

    #[cfg(test)]
    pub(crate) fn dispatch_transactional_edit_for_test(
        &mut self,
        command: crate::input::EditCommand,
    ) -> AppEffect {
        let intent = crate::edit_transaction::edit_intent_for_command(&command)
            .expect("Command should resolve to EditIntent");
        self.dispatch_transactional_edit(intent, None)
    }
}

#[cfg(test)]
mod edit_tests {

    use super::*;
    use crate::app_dispatch::canvas_drag_test_support::{
        app_with_canvas_drag_tabs, cancel_request_count, document_texts, start_canvas_drag,
    };
    use crate::document_view::DocumentView;
    use crate::input::EditCommand;

    fn execute_default_transaction_for_editor_test(
        command: &EditCommand,
        document: &mut DocumentView,
    ) -> crate::edit_transaction::TransactionalEditOutcome {
        let intent = match command {
            EditCommand::InsertChar(text) | EditCommand::InsertText(text) => {
                ui::plugin::EditIntent::InsertText(text.clone())
            }
            EditCommand::InsertNewline => ui::plugin::EditIntent::InsertParagraphBreak,
            EditCommand::Backspace => ui::plugin::EditIntent::DeleteBackward,
            EditCommand::DeleteForward => ui::plugin::EditIntent::DeleteForward,
            _ => panic!("editor transaction test requires a text edit command"),
        };
        let request = crate::edit_transaction::build_edit_request(document, intent);
        let plan = crate::edit_transaction::default_edit_plan(&request, document);

        crate::edit_transaction::execute_edit_plan(plan, document)
            .expect("default transaction plan must execute for supported editor test commands")
    }

    #[test]
    fn insert_char_outcome_executed() {
        let mut dv = DocumentView::new(vec!["hello".to_string()], 40, 40.0);
        let outcome = execute_default_transaction_for_editor_test(
            &EditCommand::InsertChar("x".into()),
            &mut dv,
        );
        assert!(outcome.edit_outcome.executed, "InsertChar should execute");
    }

    #[test]
    fn wysiwyg_text_edit_clears_vertical_navigation_preferred_x() {
        use crate::dispatch::wysiwyg::semantic_test_support::{
            SemanticPluginState, app_with_semantic_plugin,
        };
        use std::cell::RefCell;
        use std::rc::Rc;

        let state = Rc::new(RefCell::new(SemanticPluginState::default()));
        let mut app = app_with_semantic_plugin("hello", state);
        app.editor_runtime.set_preferred_x(Some(120.0));

        let effect =
            app.dispatch_transactional_edit_for_test(EditCommand::InsertText(String::from("x")));

        assert!(effect.redraw, "text edit should redraw the WYSIWYG editor");
        assert_eq!(
            app.editor_runtime.preferred_x(),
            None,
            "editing must discard the horizontal column captured by earlier vertical movement"
        );
    }

    #[test]
    #[cfg(feature = "markdown")]
    fn markdown_semantic_edit_wraps_the_selection_in_one_transaction() {
        let mut app = App::new(None);
        let mut document = DocumentView::new(vec!["hello".to_owned()], 40, 40.0);
        document.cursor_move_to_offset("hello".len());
        document.cursor_mut().selection_anchor = Some(0);
        app.push_entry_for_test(
            document,
            Box::new(textora_markdown::view::MarkdownEditorView::new()),
        );
        app.switch_workspace_for_test(0);

        let effect = app.dispatch_semantic_edit(ui::plugin::SemanticEditCommand::ToggleBold);

        let tab = app.active_tab_session().expect("Markdown tab should remain active");
        assert_eq!(tab.full_text(), "**hello**");
        assert!(tab.document.dirty);
        assert!(effect.redraw);
    }

    #[test]
    fn backspace_outcome_executed() {
        let mut dv = DocumentView::new(vec!["hello".to_string()], 40, 40.0);
        dv.cursor_move_to_offset(5);
        let outcome = execute_default_transaction_for_editor_test(&EditCommand::Backspace, &mut dv);
        assert!(outcome.edit_outcome.executed, "Backspace should execute");
    }

    #[test]
    fn delete_forward_outcome_executed() {
        let mut dv = DocumentView::new(vec!["hello".to_string()], 40, 40.0);
        let outcome =
            execute_default_transaction_for_editor_test(&EditCommand::DeleteForward, &mut dv);
        assert!(outcome.edit_outcome.executed, "DeleteForward should execute");
    }

    #[test]
    fn visible_search_panel_only_receives_edit_commands_when_focused() {
        assert!(!search_panel_receives_edit_commands(true, KeyboardFocusTarget::Editor));
        assert!(search_panel_receives_edit_commands(
            true,
            KeyboardFocusTarget::Widget(ui::core::widget::ids::SEARCH_BAR)
        ));
    }

    #[test]
    fn insert_char_produces_dirty_lines() {
        let mut dv = DocumentView::new(vec!["hello".to_string()], 40, 40.0);
        let outcome = execute_default_transaction_for_editor_test(
            &EditCommand::InsertChar("x".into()),
            &mut dv,
        );
        assert!(
            outcome.edit_outcome.dirty_lines.is_some(),
            "InsertChar should produce dirty lines"
        );
    }

    #[test]
    fn same_line_insert_char_only_needs_redraw() {
        let mut dv = DocumentView::new(vec!["hello".to_string()], 40, 40.0);
        let outcome = execute_default_transaction_for_editor_test(
            &EditCommand::InsertChar("x".into()),
            &mut dv,
        );

        assert!(!edit_requires_reshape(
            &EditCommand::InsertChar("x".into()),
            &outcome.edit_outcome
        ));
    }

    #[test]
    fn insert_newline_still_needs_reshape() {
        let mut dv = DocumentView::new(vec!["hello".to_string()], 40, 40.0);
        let outcome =
            execute_default_transaction_for_editor_test(&EditCommand::InsertNewline, &mut dv);

        assert!(edit_requires_reshape(&EditCommand::InsertNewline, &outcome.edit_outcome));
    }

    #[test]
    fn both_paste_commands_require_reshape_without_line_count_change() {
        let same_line_edit = EditOutcome {
            executed: true,
            dirty_lines: Some(0..1),
            old_line_count: 1,
            new_line_count: 1,
        };

        for command in [EditCommand::Paste, EditCommand::PastePlainText] {
            assert!(edit_requires_reshape(&command, &same_line_edit), "{command:?}");
        }
    }

    #[test]
    fn wysiwyg_navigation_predicate_covers_visual_movement() {
        assert!(is_wysiwyg_navigation_command(&EditCommand::MoveRight));
        assert!(is_wysiwyg_navigation_command(&EditCommand::MoveDown));
        assert!(is_wysiwyg_navigation_command(&EditCommand::PageUp));
        assert!(is_wysiwyg_navigation_command(&EditCommand::ExtendToDocEnd));
    }

    #[test]
    fn wysiwyg_navigation_predicate_rejects_edits_and_word_navigation() {
        assert!(!is_wysiwyg_navigation_command(&EditCommand::MoveWordLeft));
        assert!(!is_wysiwyg_navigation_command(&EditCommand::MoveWordRight));
        assert!(!is_wysiwyg_navigation_command(&EditCommand::InsertNewline));
        assert!(!is_wysiwyg_navigation_command(&EditCommand::Backspace));
        assert!(!is_wysiwyg_navigation_command(&EditCommand::InsertChar("x".into())));
        assert!(!is_wysiwyg_navigation_command(&EditCommand::InsertText("中".into())));
    }

    #[test]
    fn cursor_navigation_predicate_covers_caret_moving_commands() {
        for command in [
            EditCommand::MoveLeft,
            EditCommand::MoveRight,
            EditCommand::MoveUp,
            EditCommand::MoveDown,
            EditCommand::MoveWordLeft,
            EditCommand::MoveWordRight,
            EditCommand::MoveToLineStart,
            EditCommand::MoveToLineEnd,
            EditCommand::MoveToDocStart,
            EditCommand::MoveToDocEnd,
            EditCommand::PageUp,
            EditCommand::PageDown,
            EditCommand::ExtendLeft,
            EditCommand::ExtendRight,
            EditCommand::ExtendUp,
            EditCommand::ExtendDown,
            EditCommand::ExtendWordLeft,
            EditCommand::ExtendWordRight,
            EditCommand::ExtendToLineStart,
            EditCommand::ExtendToLineEnd,
            EditCommand::ExtendToDocStart,
            EditCommand::ExtendToDocEnd,
            EditCommand::SelectAll,
            EditCommand::NavigateBack,
            EditCommand::NavigateForward,
        ] {
            assert!(is_cursor_navigation_command(&command), "{command:?} must break edit merge");
        }
    }

    #[test]
    fn cursor_navigation_predicate_rejects_editing_and_panel_commands() {
        for command in [
            EditCommand::InsertChar("x".into()),
            EditCommand::InsertText("中".into()),
            EditCommand::InsertNewline,
            EditCommand::Backspace,
            EditCommand::DeleteForward,
            EditCommand::Tab,
            EditCommand::Undo,
            EditCommand::Redo,
            EditCommand::Cut,
            EditCommand::Copy,
            EditCommand::Paste,
            EditCommand::Find,
            EditCommand::Escape,
        ] {
            assert!(
                !is_cursor_navigation_command(&command),
                "{command:?} must not break edit merge"
            );
        }
    }

    #[test]
    fn cursor_navigation_between_typing_splits_undo_entries() {
        use crate::plugins::editor::EditorPlugin;

        let mut app = App::new(None);
        let dv = DocumentView::new(vec![String::new()], 40, 40.0);
        app.push_entry_for_test(dv, Box::new(EditorPlugin::new()));
        app.switch_workspace_for_test(0);

        app.dispatch_transactional_edit_for_test(EditCommand::InsertText("a".into()));
        // The caret leaves and returns to the exact byte where typing ended.
        app.break_edit_merge_for_navigation(&EditCommand::MoveLeft);
        {
            let Some(tab) = app.active_tab_session_mut() else {
                panic!("source editor tab should be active");
            };
            tab.document.cursor_move_left();
        }
        app.break_edit_merge_for_navigation(&EditCommand::MoveRight);
        {
            let Some(tab) = app.active_tab_session_mut() else {
                panic!("source editor tab should be active");
            };
            tab.document.cursor_move_right();
        }
        app.dispatch_transactional_edit_for_test(EditCommand::InsertText("b".into()));

        let Some(tab) = app.active_tab_session_mut() else {
            panic!("source editor tab should be active");
        };
        assert_eq!(tab.document.full_text(), "ab");
        tab.document.undo();
        assert_eq!(
            tab.document.full_text(),
            "a",
            "first undo must only remove the text typed after navigating"
        );
        tab.document.undo();
        assert_eq!(tab.document.full_text(), "");
    }

    #[test]
    fn keyboard_tab_switch_commands_cancel_started_canvas_drag_once() {
        for command in [EditCommand::NextTab, EditCommand::PrevTab, EditCommand::SwitchTab(1)] {
            let (mut app, state) = app_with_canvas_drag_tabs();
            start_canvas_drag(&mut app);

            let effect = app.dispatch_keyboard_tab_switch(&command);

            assert_eq!(app.active_editor_index(), Some(1));
            assert!(effect.redraw);
            assert_eq!(cancel_request_count(&state.borrow()), 1);
            assert_eq!(document_texts(&app), ["abc", "def"]);

            app.dispatch_keyboard_tab_switch(&command);

            assert_eq!(cancel_request_count(&state.borrow()), 1);
        }
    }

    #[test]
    fn keyboard_tab_switch_is_noop_without_an_active_tab() {
        let mut app = App::new(None);

        assert_eq!(app.dispatch_keyboard_tab_switch(&EditCommand::NextTab), AppEffect::NONE);
        assert_eq!(app.dispatch_keyboard_tab_switch(&EditCommand::PrevTab), AppEffect::NONE);
    }
}
