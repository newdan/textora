//! Event handling helpers — keyboard, mouse, and scroll event processing.
//!
//! Extracted from `app.rs` to separate event dispatch from application lifecycle.
//!
//! Architecture (Phase 6.2 unified routing):
//!   Each mouse handler calls `ui_shell.dispatch()` **once**.
//!   `UiShell::dispatch` routes: overlays → Dock (tab_bar, search_bar, status_bar,
//!   sidebar, scrollbar, editor fill) using each widget's `hit()` method.
//!   The handler then translates the resulting `WidgetAction` into `AppAction`s.

use winit::event::{ElementState, MouseScrollDelta};
use winit::keyboard::{ModifiersState, PhysicalKey};
use winit::window::CursorIcon;

use crate::actions::AppAction;
use crate::app::App;
use crate::app_effect::AppEffect;
use crate::input::key_to_command;
use crate::menu_handler::AppCommand;
use crate::mouse::hit_test_with_sub_line_offset as mouse_hit_test;
use appkit_core::workspace::types::TabId;
use appkit_shell::editor_runtime::MouseCapture;
use appkit_shell::window_input::{
    command_allowed_during_preedit, is_ime_process_key, ui_modifiers,
};
use ui::core::widget::{Event, EventCtx, MouseButton as WidgetMouseButton, WidgetAction};
use ui::plugin::EditHitTarget;

fn mmap_cursor_icon(app: &mut App, px: f32, py: f32) -> Option<CursorIcon> {
    let is_mmap = app.active_tab_session().is_some_and(|tab| {
        tab.plugin_name() == ui::plugin::PLUGIN_MINDMAP
            && tab.handles_own_rendering()
            && tab.allows_editing()
    });
    if !is_mmap {
        return None;
    }

    if app.editor_runtime.pointer_capture() == MouseCapture::CanvasDrag
        || app.mouse.canvas_drag.as_ref().is_some_and(|session| session.started)
    {
        return Some(CursorIcon::Grabbing);
    }

    let target = app.query_plugin_edit_hit_target(px, py)?;
    match target {
        Some(EditHitTarget::TextCaret { .. }) => Some(CursorIcon::Text),
        Some(EditHitTarget::SourceObject { .. }) => Some(CursorIcon::Grab),
        Some(EditHitTarget::CanvasControl { .. }) => Some(CursorIcon::Pointer),
        Some(EditHitTarget::ClearFocus) | None => Some(CursorIcon::Default),
    }
}

// ── Keyboard ──────────────────────────────────────────────────────────────

fn markdown_semantic_shortcut(
    physical_key: PhysicalKey,
    modifiers: ModifiersState,
) -> Option<ui::plugin::SemanticEditCommand> {
    appkit_shell::window_input::markdown_semantic_shortcut(physical_key, ui_modifiers(modifiers))
}

fn is_toggle_pin_shortcut(key_text: Option<&str>, modifiers: ModifiersState) -> bool {
    modifiers.super_key() && modifiers.shift_key() && matches!(key_text, Some("p" | "P"))
}

/// Handle keyboard input events.
pub(crate) fn handle_keyboard(app: &mut App, event: &winit::event::KeyEvent) -> Vec<AppAction> {
    let mut actions = Vec::new();

    // ── IME 互斥守卫 ────────────────────────────────────────────
    // macOS + winit 0.30 会同时派发 Ime::Preedit 和 KeyboardInput。
    // IME composition 期间禁止文档修改走 KeyboardInput 路径，
    // 让 Ime::Commit 作为输入提交的唯一入口，避免双插入或提前删除。
    //
    // 条件 1：平台明确标记此键已被 IME 消费。
    if is_ime_process_key(&event.logical_key) {
        return actions;
    }

    let key_code =
        crate::app_lifecycle::winit_key_to_keycode(&event.logical_key, event.text.as_deref());
    let input_modifiers = app.editor_runtime.input_modifiers();
    let modifiers = ui_modifiers(input_modifiers);

    // 条件 2：当前正处于 IME composition（preedit_text 非空）。
    // 必须先于插件快捷键映射检查，否则 Enter / Backspace 会通过
    // EditIntent 直接修改文档。导航与非编辑快捷键仍可继续执行。
    let (preedit_text, _) = app.editor_runtime.preedit();
    if !preedit_text.is_empty() {
        if let Some(command) = key_to_command(&event.logical_key, input_modifiers)
            && command_allowed_during_preedit(&preedit_text, &command)
        {
            actions.push(AppAction::ExecuteAppCommands(vec![AppCommand::Edit(command)]));
        }
        return actions;
    }

    let markdown_editor_is_focused = app.active_plugin_name()
        == Some(ui::plugin::PLUGIN_MARKDOWN_EDITOR)
        && app.active_allows_editing()
        && app.ui_shell.keyboard_focus() == crate::ui_shell::KeyboardFocusTarget::Editor;
    if markdown_editor_is_focused
        && let Some(command) = markdown_semantic_shortcut(event.physical_key, input_modifiers)
    {
        let effect = app.dispatch_semantic_edit(command);
        app.apply_effect(effect);
        return Vec::new();
    }

    let mapped_intent = key_code.as_ref().and_then(|key_code| {
        app.active_tab_session().and_then(|tab| tab.map_key_intent(key_code, &modifiers))
    });
    if let Some(intent) = mapped_intent {
        let effect = app.dispatch_transactional_edit(intent, None);
        app.apply_effect(effect);
        return Vec::new();
    }

    let key_text = event.logical_key.to_text();
    if is_toggle_pin_shortcut(key_text, input_modifiers) {
        actions.push(AppAction::TogglePin);
        return actions;
    }
    #[allow(clippy::collapsible_if)]
    if let Some(mut tab) = app.active_tab_session_mut() {
        if let Some(key_code) = key_code.as_ref() {
            if tab.intercept_key(key_code, &modifiers) {
                return vec![AppAction::RequestRedraw];
            }
        }
    }

    let fallback_cmd = key_to_command(&event.logical_key, input_modifiers);

    // Reading mode: Space acts as PageDown instead of inserting a character.
    let is_reading_mode = app.active_is_reading_mode();
    let fallback_cmd = if is_reading_mode {
        match fallback_cmd {
            Some(crate::input::EditCommand::InsertChar(s)) if s == " " => {
                Some(crate::input::EditCommand::PageDown)
            }
            other => other,
        }
    } else {
        fallback_cmd
    };

    if let Some(cmd) = fallback_cmd {
        actions.push(AppAction::ExecuteAppCommands(vec![AppCommand::Edit(cmd)]));
    }
    actions
}

/// Handle mouse scroll events.
pub(crate) fn handle_scroll(_app: &App, delta: MouseScrollDelta) -> Vec<AppAction> {
    vec![AppAction::HandleScroll(delta)]
}

/// Unified dispatch: build event, call ui_shell.dispatch(), translate result.
/// Returns (actions, consumed) — `consumed` is true when a widget handled the
/// event and it should not fall through to the editor.
fn dispatch_mouse(app: &mut App, ev: Event) -> (Vec<AppAction>, bool, Option<CursorIcon>) {
    let metrics = app.ui_metrics();
    let mut ctx = EventCtx::new(&app.current_theme, metrics.dpi);
    let mut actions = Vec::new();

    let widget_action = app.ui_shell.dispatch(&ev, &mut ctx);

    if let Some(action) = widget_action {
        translate_widget_action(&action, app, &mut actions);
        // Clicking a keyboard-aware widget moves focus to it (MouseDown only).
        if matches!(&ev, Event::MouseDown { .. }) {
            let focus_id = match &action {
                WidgetAction::SearchBar(_) => Some(ui::core::widget::ids::SEARCH_BAR),
                WidgetAction::MindmapStylePanel(_) => {
                    Some(ui::core::widget::ids::MINDMAP_STYLE_PANEL)
                }
                _ => None,
            };
            if let Some(focus_id) = focus_id {
                app.ui_shell.focus_widget(focus_id);
            }
        }
        let consumed = widget_action_consumes_editor_fallthrough(&action);
        let cursor_hint = ctx.cursor_hint;
        if let Some(sync_action) = app.take_pending_sync_settings_action() {
            actions.push(AppAction::Sync(sync_action));
        }
        return (actions, consumed, cursor_hint);
    }

    (actions, false, ctx.cursor_hint)
}

fn dispatch_lifecycle_to_ui(app: &mut App, event: Event) {
    let metrics = app.ui_metrics();
    let mut context = EventCtx::new(&app.current_theme, metrics.dpi);
    let _ = app.ui_shell.dispatch(&event, &mut context);
}

pub(crate) fn handle_pointer_leave(app: &mut App) -> Vec<AppAction> {
    dispatch_lifecycle_to_ui(app, Event::PointerLeave);
    app.mouse.last_hover_redraw_pos = None;

    let mut actions = Vec::new();
    if app.mouse.last_hover_tab.take().is_some() {
        actions.push(AppAction::HoverTab(None));
    }
    actions.push(AppAction::SetCursor(CursorIcon::Default));
    actions.push(AppAction::RequestRedraw);
    actions
}

pub(crate) fn handle_interaction_cancel(app: &mut App) -> AppEffect {
    dispatch_lifecycle_to_ui(app, Event::InteractionCancel);
    let effect = app.cancel_canvas_drag();
    app.editor_runtime.focus_lost();
    app.mouse.is_down = false;
    app.mouse.down_byte_offset = None;
    app.mouse.wysiwyg_selection_scope = None;
    app.mouse.last_hover_redraw_pos = None;
    app.mouse.last_hover_tab = None;
    if let Some(window) = app.editor_runtime.window() {
        window.set_cursor(CursorIcon::Default);
    }
    effect.merge(AppEffect::REDRAW)
}

/// Translate a single WidgetAction into AppActions.
pub(crate) fn translate_widget_action(
    action: &WidgetAction,
    app: &App,
    actions: &mut Vec<AppAction>,
) {
    match action {
        WidgetAction::Control(_) => {}
        WidgetAction::Overlay(overlay_action) => translate_overlay_action(overlay_action, actions),
        WidgetAction::Settings(settings_action) => {
            actions.push(AppAction::Settings(settings_action.clone()));
        }
        WidgetAction::Sidebar(sa) => translate_sidebar_action(app, sa, actions),
        WidgetAction::TabBar(ta) => translate_tab_action(ta, app, actions),
        WidgetAction::Scrollbar(sa) => translate_scrollbar_action(sa, app, actions),
        WidgetAction::CanvasScrollbars(action) => {
            translate_canvas_scrollbar_action(action, actions)
        }
        WidgetAction::SearchBar(sa) => translate_search_action(sa, actions),
        WidgetAction::Popup(outcome) => translate_popup_outcome(outcome, app, actions),
        WidgetAction::List(_) => {}
        WidgetAction::TreeList(_) => {}
        WidgetAction::VirtualCardList(_) => {}
        WidgetAction::Splitter(_) => {}
        WidgetAction::TitleBar(ta) => translate_title_bar_action(ta, actions),
        WidgetAction::Toc(ta) => translate_toc_action(ta, actions),
        WidgetAction::MindmapStylePanel(action) => {
            actions.push(AppAction::MindmapStylePanel(action.clone()));
        }
        WidgetAction::Consumed => {}
    }
}

fn translate_overlay_action(
    action: &ui::core::overlay::OverlayAction,
    actions: &mut Vec<AppAction>,
) {
    match action {
        ui::core::overlay::OverlayAction::DismissRequested => {
            actions.push(AppAction::DismissOverlay);
        }
    }
}

fn widget_action_consumes_editor_fallthrough(action: &WidgetAction) -> bool {
    matches!(
        action,
        WidgetAction::Control(_)
            | WidgetAction::Overlay(_)
            | WidgetAction::Settings(_)
            | WidgetAction::Sidebar(_)
            | WidgetAction::TabBar(_)
            | WidgetAction::Scrollbar(_)
            | WidgetAction::CanvasScrollbars(_)
            | WidgetAction::SearchBar(_)
            | WidgetAction::Popup(_)
            | WidgetAction::List(_)
            | WidgetAction::TitleBar(_)
            | WidgetAction::MindmapStylePanel(_)
            | WidgetAction::Consumed
    )
}

/// Translate TitleBar widget actions into AppActions.
fn translate_title_bar_action(ta: &ui::title_bar::TitleBarAction, actions: &mut Vec<AppAction>) {
    use crate::menu_handler::AppCommand;
    use ui::title_bar::TitleBarAction;
    match ta {
        TitleBarAction::ToggleView => {
            actions.push(AppAction::ExecuteAppCommands(vec![AppCommand::Edit(
                crate::input::EditCommand::ToggleView,
            )]));
        }
        TitleBarAction::ToggleToc => {
            actions.push(AppAction::ExecuteAppCommands(vec![AppCommand::Edit(
                crate::input::EditCommand::ToggleToc,
            )]));
        }
        TitleBarAction::ToggleMindmapStylePanel => {
            actions.push(AppAction::ToggleMindmapStylePanel);
        }
    }
}

/// Translate Toc widget actions into AppActions.
fn translate_toc_action(ta: &ui::toc::TocAction, actions: &mut Vec<AppAction>) {
    use ui::toc::TocAction;
    match ta {
        TocAction::JumpToHeading(idx) => {
            actions.push(AppAction::JumpToHeading(*idx));
        }
    }
}
// ── Cursor moved ──────────────────────────────────────────────────────────

/// Handle cursor movement — unified Dock dispatch, title bar guard, tab hover,
/// overlay highlight, and editor hit-test fallthrough.
pub(crate) fn handle_cursor_moved(app: &mut App, px: f32, py: f32) -> Vec<AppAction> {
    let mut actions = vec![AppAction::UpdateMousePos(px as f64, py as f64)];

    // 0. Sidebar hover state machine: feed mouse position every frame

    // 1. Unified widget dispatch (overlays → Dock)
    let (widget_actions, consumed, cursor_hint) = dispatch_mouse(app, Event::MouseMove { px, py });
    // Sync mouse.last_hover_tab from any HoverTab actions the tab bar just produced,
    // so subsequent frames can dedupe HoverTab(None) pushes (方案 2026-07-06 阶段 4a).
    for a in &widget_actions {
        if let AppAction::HoverTab(id_opt) = a {
            app.mouse.last_hover_tab = id_opt.and_then(|id| app.editor_tab_index(id));
        }
    }
    actions.extend(widget_actions);
    // Sync sidebar persistent state back from widget after Dock dispatch
    if matches!(app.settings.view_mode, ui::view_mode::ViewMode::Sidebar) {
        app.ui_shell.sync_sidebar_persistent();
    }

    // Cursor hint from widgets (sidebar edge → ColResize, title bar → Default, etc.)
    // always takes priority — cursor icon should reflect what's under the pointer,
    // even during an editor drag.
    if let Some(icon) = cursor_hint {
        actions.push(AppAction::SetCursor(icon));
    }

    // During an editor drag (mouse button held), don't let widgets consume
    // the event — the editor must receive EditorCursorMoved for selection.
    let dragging = app.mouse.is_down || app.editor_runtime.pointer_capture() != MouseCapture::None;

    if !dragging && consumed {
        // Widget consumed the event (sidebar click, scrollbar, etc.) — stop here.
        if cursor_hint.is_none() {
            actions.push(AppAction::SetCursor(CursorIcon::Default));
        }
        // Request redraw so hover effects (e.g. TOC item highlighting) are visible
        actions.push(AppAction::RequestRedraw);
        return actions;
    }

    // Clear tab hover when outside tab bar area. Dedupe: only push None when
    // last state was Some(_) — otherwise mouse move over the editor area spams
    // HoverTab(None) → chrome dispatch → REDRAW every frame (方案 2026-07-06 阶段 4a).
    if app.mouse.last_hover_tab.is_some() {
        actions.push(AppAction::HoverTab(None));
        app.mouse.last_hover_tab = None;
    }

    // 4. Editor hit-test fallthrough
    let content_top = app.content_top_offset();
    let line_count = app.active_document_line_count();
    let left_margin = app.editor_left_margin(line_count);
    let gutter_w = app.settings.gutter_width(line_count) * app.ui_metrics().dpi;
    let hit = {
        let metrics = app.ui_metrics();
        app.active_tab_session().map(|tab| (tab, metrics)).and_then(|(tab, metrics)| {
            let document = tab.document;
            let li = &document.line_index;
            mouse_hit_test(
                px,
                py,
                tab.advance_cache(),
                &metrics,
                left_margin,
                content_top,
                tab.sub_line_pixel_offset(metrics.line_height),
                li,
            )
        })
    };
    actions.push(AppAction::EditorCursorMoved { px, py, hit });
    if cursor_hint.is_none() {
        if let Some(icon) = mmap_cursor_icon(app, px, py) {
            actions.push(AppAction::SetCursor(icon));
        } else if app.active_allows_editing() {
            let bounds = app.plugin_render_bounds();
            if px < bounds.x || px > bounds.x + bounds.w {
                actions.push(AppAction::SetCursor(CursorIcon::Default));
            } else {
                actions.push(AppAction::SetCursor(CursorIcon::Text));
            }
        } else {
            let in_gutter = gutter_w > 0.0 && px >= left_margin - gutter_w && px < left_margin;
            if in_gutter {
                actions.push(AppAction::SetCursor(CursorIcon::Default));
            } else {
                actions.push(AppAction::SetCursor(CursorIcon::Text));
            }
        }
    }

    // 5. Overlay hover highlight — already routed by dispatch_mouse above;
    //    if overlays exist and consumed, we returned early.
    //    If overlays exist but didn't consume (no hit), only trigger a redraw
    //    when the mouse has moved past a jitter threshold since the last
    //    overlay-triggered redraw (方案 2026-07-06 阶段 4a). This eliminates
    //    per-frame redraw spam under a stationary cursor with any overlay open.
    if app.ui_shell.overlays_count() > 0 && app.mouse.overlay_hover_needs_redraw(px, py) {
        app.mouse.last_hover_redraw_pos = Some((px, py));
        actions.push(AppAction::RequestRedraw);
    }

    actions
}

// ── Left mouse button ─────────────────────────────────────────────────────

/// Handle left mouse button press/release — unified Dock dispatch with title
/// bar guard, tab bar, and editor fallthrough.
pub(crate) fn handle_mouse_input_left(
    app: &mut App,
    state: ElementState,
    px: f32,
    py: f32,
) -> Vec<AppAction> {
    let mut actions = Vec::new();

    // 1. Unified widget dispatch (overlays → Dock)
    let ev = if state.is_pressed() {
        Event::MouseDown { px, py, button: WidgetMouseButton::Left }
    } else {
        Event::MouseUp { px, py, button: WidgetMouseButton::Left }
    };
    let (widget_actions, consumed, cursor_hint) = dispatch_mouse(app, ev);
    actions.extend(widget_actions);
    // Sync sidebar persistent state after MouseDown/MouseUp (menu close, etc.)
    if matches!(app.settings.view_mode, ui::view_mode::ViewMode::Sidebar) {
        app.ui_shell.sync_sidebar_persistent();
    }

    if let Some(icon) = cursor_hint {
        actions.push(AppAction::SetCursor(icon));
    }

    // During an editor drag (mouse button held), don't let widgets consume
    // MouseUp — the editor must receive it to reset is_down.
    if consumed && !app.mouse.is_down && app.editor_runtime.pointer_capture() == MouseCapture::None
    {
        return actions;
    }

    // 3. Tab bar click — already handled by dispatch_mouse above;
    //    removed redundant second dispatch.

    // 4. Editor hit-test fallthrough
    // A click that reaches the editor returns keyboard focus from dock widgets.
    // Consumed panel/search events returned above and never reach this branch.
    let dock_widget_has_focus = matches!(
        app.ui_shell.keyboard_focus(),
        crate::ui_shell::KeyboardFocusTarget::Widget(id)
            if id == ui::core::widget::ids::SEARCH_BAR
                || id == ui::core::widget::ids::MINDMAP_STYLE_PANEL
    );
    if state.is_pressed() && dock_widget_has_focus {
        app.ui_shell.focus_editor();
    }
    let content_top = app.content_top_offset();
    let hit = {
        let metrics = app.ui_metrics();
        app.active_tab_session().map(|tab| (tab, metrics)).and_then(|(tab, metrics)| {
            let document = tab.document;
            let left_margin = app.editor_left_margin(document.line_count());
            let li = &document.line_index;
            mouse_hit_test(
                px,
                py,
                tab.advance_cache(),
                &metrics,
                left_margin,
                content_top,
                tab.sub_line_pixel_offset(metrics.line_height),
                li,
            )
        })
    };
    actions.push(AppAction::EditorMouseInput { state, px, py, hit });
    actions
}

// ── Right mouse button ────────────────────────────────────────────────────

/// Handle right mouse button press — unified Dock dispatch with title bar
/// guard, tab bar, and editor fallthrough.
pub(crate) fn handle_mouse_input_right(
    app: &mut App,
    state: ElementState,
    px: f32,
    py: f32,
) -> Vec<AppAction> {
    let mut actions = Vec::new();
    if !state.is_pressed() {
        return actions;
    }

    // 1. Unified widget dispatch (overlays → Dock)
    let ev = Event::MouseDown { px, py, button: WidgetMouseButton::Right };
    let (widget_actions, consumed, cursor_hint) = dispatch_mouse(app, ev);
    actions.extend(widget_actions);

    if let Some(icon) = cursor_hint {
        actions.push(AppAction::SetCursor(icon));
    }

    if consumed {
        return actions;
    }

    // 3. Tab bar right-click — already handled by dispatch_mouse above;
    //    removed redundant second dispatch.

    actions
}

// ── Translation helpers ───────────────────────────────────────────────────

fn translate_scrollbar_action(
    sa: &ui::scrollbar::ScrollbarAction,
    app: &App,
    actions: &mut Vec<AppAction>,
) {
    use ui::scrollbar::ScrollbarAction as SA;
    match sa {
        SA::DragTo(scroll) => {
            actions.push(AppAction::UpdateScrollTop(*scroll));
        }
        SA::HoverChanged(is_over) => {
            if *is_over {
                actions.push(AppAction::SetCursor(CursorIcon::Default));
            }
            // State change needs visual update
            actions.push(AppAction::RequestRedraw);
        }
        SA::PageUp => {
            let amount = if app.active_handles_own_rendering() {
                -1.0
            } else if let Some(tab) = app.active_tab_session() {
                -tab.viewport_height()
            } else {
                return;
            };
            actions.push(AppAction::ScrollViewportBy(amount));
        }
        SA::PageDown => {
            let amount = if app.active_handles_own_rendering() {
                1.0
            } else if let Some(tab) = app.active_tab_session() {
                tab.viewport_height()
            } else {
                return;
            };
            actions.push(AppAction::ScrollViewportBy(amount));
        }
        SA::StartDrag => {
            actions.push(AppAction::RequestRedraw);
        }
        SA::EndDrag => {
            actions.push(AppAction::RequestRedraw);
        }
    }
}

fn translate_canvas_scrollbar_action(
    action: &ui::canvas_scrollbars::CanvasScrollbarsAction,
    actions: &mut Vec<AppAction>,
) {
    actions.push(AppAction::CanvasScrollbar { axis: action.axis, action: action.action.clone() });
}

fn translate_search_action(sa: &ui::search_bar::SearchBarAction, actions: &mut Vec<AppAction>) {
    use ui::search_bar::SearchBarAction as SA;
    match sa {
        SA::QueryChanged(_)
        | SA::ReplaceQueryChanged(_)
        | SA::Next
        | SA::Prev
        | SA::Close
        | SA::DismissOrClear
        | SA::ToggleReplace
        | SA::ToggleRegex
        | SA::Replace
        | SA::ReplaceAll
        | SA::FocusFind
        | SA::FocusReplace
        | SA::HoverChanged => {
            actions.push(AppAction::SearchBarAction(sa.clone()));
        }
    }
}

fn tab_id_for_index(app: &App, index: usize) -> Option<TabId> {
    app.editor_tab_id_at(index)
}

fn popup_tab_id_for_index(app: &App, index: usize) -> Option<TabId> {
    app.popup_tab_id_for_index(index)
}

fn translate_popup_outcome(
    outcome: &ui::popup_menu::PopupOutcome,
    app: &App,
    actions: &mut Vec<AppAction>,
) {
    match outcome {
        ui::popup_menu::PopupOutcome::Selected(pm_action) => {
            translate_popup_action(pm_action, app, actions);
            actions.push(AppAction::ClearPopupMenu);
        }
        ui::popup_menu::PopupOutcome::Dismiss => {
            actions.push(AppAction::ClearPopupMenu);
        }
    }
}

fn translate_popup_action(
    action: &ui::popup_menu::PopupMenuAction,
    app: &App,
    actions: &mut Vec<AppAction>,
) {
    use ui::popup_menu::PopupMenuAction as PMA;
    match action {
        PMA::SwitchTab(idx) => {
            if let Some(id) = popup_tab_id_for_index(app, *idx) {
                actions.push(AppAction::SwitchTab(id));
            }
        }
        PMA::Context { action: ctx_action, tab_index } => {
            if let Some(id) = popup_tab_id_for_index(app, *tab_index) {
                actions.push(AppAction::ExecuteContextMenuAction(*ctx_action, id));
            }
        }
        PMA::SetViewMode(mode) => actions.push(AppAction::SetViewMode(*mode)),
        PMA::OpenSettingsFile => actions.push(AppAction::OpenSettingsFile),
        PMA::ToggleLineNumbers => actions.push(AppAction::ToggleLineNumbers),
        PMA::ToggleWordWrap => actions.push(AppAction::ToggleWordWrap),
        PMA::ToggleStatusBar => actions.push(AppAction::ToggleStatusBar),
        PMA::SetThemeMode(mode) => actions.push(AppAction::SetThemeMode(*mode)),
        PMA::NewDocument(kind) => actions.push(AppAction::NewDocument(*kind)),
    }
}

fn translate_tab_action(ta: &ui::tab_bar::TabBarAction, app: &App, actions: &mut Vec<AppAction>) {
    use ui::tab_bar::TabBarAction as TA;
    match ta {
        TA::SwitchTab(idx) => {
            if let Some(id) = tab_id_for_index(app, *idx) {
                actions.push(AppAction::SwitchTab(id));
            }
        }
        TA::CloseTab(idx) => {
            if let Some(id) = tab_id_for_index(app, *idx) {
                actions.push(AppAction::CloseTab(id));
            }
        }
        TA::NewEmptyTab => actions.push(AppAction::NewEmptyTab),
        TA::ScrollLeft => actions.push(AppAction::ScrollTabLeft),
        TA::ScrollRight => actions.push(AppAction::ScrollTabRight),
        TA::NavigateBack => {}
        TA::NavigateForward => {}
        TA::OpenOverflowMenu => actions.push(AppAction::OpenPopupOverflow),
        TA::OpenContextMenuPx { tab_index, anchor_px } => {
            let is_pinned = app.is_editor_tab_pinned_at(*tab_index);
            let screen_w = app.screen_width();
            let screen_h = app.screen_height();
            let dpi = app.ui_metrics().dpi;
            let pm = ui::popup_menu::PopupMenu::context_px(
                *tab_index,
                *anchor_px,
                (screen_w, screen_h),
                is_pinned,
                dpi,
            );
            actions.push(AppAction::OpenPopupMenu(pm));
        }
        TA::Context { action: ctx_action, tab_index } => {
            if let Some(id) = tab_id_for_index(app, *tab_index) {
                actions.push(AppAction::ExecuteContextMenuAction(*ctx_action, id));
            }
        }
        TA::HoverTab(idx_opt) => {
            actions.push(AppAction::HoverTab(idx_opt.and_then(|i| tab_id_for_index(app, i))))
        }
    }
}

fn translate_sidebar_action(
    app: &App,
    sa: &ui::sidebar::SidebarAction,
    actions: &mut Vec<AppAction>,
) {
    use ui::sidebar::SidebarAction as S;
    match sa {
        S::SwitchTab(idx) => {
            if let Some(id) = tab_id_for_index(app, *idx) {
                actions.push(AppAction::SwitchTab(id));
            }
        }
        S::CloseTab(idx) => {
            if let Some(id) = tab_id_for_index(app, *idx) {
                actions.push(AppAction::CloseTab(id));
            }
        }
        S::NewDocument(kind) => actions.push(AppAction::NewDocument(*kind)),
        S::OpenNewDocumentMenu => {}
        S::OpenDocument => actions.push(AppAction::ExecuteAppCommands(vec![
            crate::menu_handler::AppCommand::OpenFileDialog,
        ])),
        S::ToggleViewMode => {
            // Toggle between Sidebar and Tabs
            let current = app.settings.view_mode;
            let next = match current {
                ui::view_mode::ViewMode::Sidebar => ui::view_mode::ViewMode::Tabs,
                ui::view_mode::ViewMode::Tabs => ui::view_mode::ViewMode::Sidebar,
            };
            actions.push(AppAction::SetViewMode(next));
        }
        S::TogglePin => actions.push(AppAction::ToggleSidebarPin),
        S::SetWidth(w) => {
            actions.push(AppAction::SetSidebarWidth(*w));
            actions.push(AppAction::RequestRedraw);
        }
        S::Context { action: ctx_action, tab_index } => {
            if let Some(id) = tab_id_for_index(app, *tab_index) {
                actions.push(AppAction::ExecuteContextMenuAction(*ctx_action, id));
            }
        }
        S::OpenSettingsMenu => {
            actions.push(AppAction::OpenSidebarSettingsMenu);
        }
        S::Hovered => {
            actions.push(AppAction::RequestRedraw);
        }
        S::StartResize => actions.push(AppAction::SidebarResizeStart),
        S::ResizeTo(w) => {
            actions.push(AppAction::SetSidebarWidth(*w));
            actions.push(AppAction::RequestRedraw);
        }
        S::EndResize => {
            actions.push(AppAction::SidebarResizeEnd);
        }
        S::PersistConfig => {}
        S::SetViewMode(mode) => actions.push(AppAction::SetViewMode(*mode)),
        S::OpenSettingsFile => actions.push(AppAction::OpenSettingsFile),
        S::ToggleLineNumbers => actions.push(AppAction::ToggleLineNumbers),
        S::ToggleWordWrap => actions.push(AppAction::ToggleWordWrap),
        S::ToggleStatusBar => actions.push(AppAction::ToggleStatusBar),
        S::SetThemeMode(mode) => actions.push(AppAction::SetThemeMode(*mode)),
        S::ContextMenuPx { tab_index, anchor_px, screen_size } => {
            let _ndc_x = anchor_px.0 / screen_size.0 * 2.0 - 1.0;
            let dpi = app.ui_metrics().dpi;
            let pm = ui::popup_menu::PopupMenu::context_px(
                *tab_index,
                *anchor_px,
                *screen_size,
                false,
                dpi,
            );
            actions.push(AppAction::OpenPopupMenu(pm));
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::document_view::DocumentView;
    use crate::sync_settings_types::SyncSettingsInput;
    use crate::textora_settings_overlay::TextoraSettingsOverlay;
    use crate::workspace::ViewportDimensions;
    use crate::workspace_tab_factory::ProductPreparedTab;

    use std::cell::RefCell;
    use std::rc::Rc;
    use ui::plugin::{EditHitTarget, PluginQuery, PluginResponse, ViewPlugin};
    use winit::window::CursorIcon;

    #[test]
    fn toggle_pin_shortcut_accepts_the_uppercase_character_produced_by_shift() {
        assert!(is_toggle_pin_shortcut(Some("P"), ModifiersState::SUPER | ModifiersState::SHIFT));
        assert!(!is_toggle_pin_shortcut(Some("P"), ModifiersState::SUPER));
    }

    #[test]
    fn markdown_semantic_shortcuts_cover_common_inline_and_block_commands() {
        use ui::plugin::SemanticEditCommand;
        use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

        let primary = ModifiersState::SUPER;
        let primary_shift = primary | ModifiersState::SHIFT;
        let primary_alt = primary | ModifiersState::ALT;
        let cases = [
            (KeyCode::KeyB, primary, SemanticEditCommand::ToggleBold),
            (KeyCode::KeyI, primary, SemanticEditCommand::ToggleItalic),
            (KeyCode::KeyK, primary, SemanticEditCommand::InsertLink),
            (KeyCode::KeyE, primary, SemanticEditCommand::ToggleInlineCode),
            (KeyCode::KeyX, primary_shift, SemanticEditCommand::ToggleStrikethrough),
            (KeyCode::Digit7, primary_shift, SemanticEditCommand::OrderedList),
            (KeyCode::Digit8, primary_shift, SemanticEditCommand::UnorderedList),
            (KeyCode::Digit9, primary_shift, SemanticEditCommand::Quote),
            (KeyCode::Digit1, primary_alt, SemanticEditCommand::SetHeadingLevel(1)),
            (KeyCode::Digit6, primary_alt, SemanticEditCommand::SetHeadingLevel(6)),
            (KeyCode::KeyT, primary_alt, SemanticEditCommand::TaskList),
            (KeyCode::KeyC, primary_alt, SemanticEditCommand::CodeBlock),
        ];

        for (key_code, modifiers, expected) in cases {
            assert_eq!(
                markdown_semantic_shortcut(PhysicalKey::Code(key_code), modifiers),
                Some(expected),
                "unexpected mapping for {modifiers:?}+{key_code:?}"
            );
        }
    }

    #[test]
    fn markdown_semantic_shortcuts_support_control_and_reject_near_misses() {
        use ui::plugin::SemanticEditCommand;
        use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};

        assert_eq!(
            markdown_semantic_shortcut(PhysicalKey::Code(KeyCode::KeyB), ModifiersState::CONTROL),
            Some(SemanticEditCommand::ToggleBold)
        );
        assert_eq!(
            markdown_semantic_shortcut(PhysicalKey::Code(KeyCode::KeyB), ModifiersState::empty()),
            None
        );
        assert_eq!(
            markdown_semantic_shortcut(
                PhysicalKey::Code(KeyCode::KeyB),
                ModifiersState::SUPER | ModifiersState::SHIFT
            ),
            None
        );
        assert_eq!(
            markdown_semantic_shortcut(
                PhysicalKey::Code(KeyCode::KeyI),
                ModifiersState::SUPER | ModifiersState::ALT
            ),
            None
        );
    }

    fn open_untitled_fixture(app: &mut App) {
        let dimensions = ViewportDimensions { visible_rows: 22, viewport_height: 22.0 };
        let ProductPreparedTab { prepared, suggested_file_name } =
            app.prepare_editor_untitled(dimensions);
        let _ = app.install_editor_tab(
            prepared,
            suggested_file_name,
            appkit_shell::editor_runtime::OpenDisposition::Persistent,
        );
    }

    struct MmapCursorTestPlugin {
        target: Rc<RefCell<Option<EditHitTarget>>>,
    }

    impl ViewPlugin for MmapCursorTestPlugin {
        fn name(&self) -> &str {
            ui::plugin::PLUGIN_MINDMAP
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
            match query {
                PluginQuery::HitTestEditTarget { .. } => {
                    PluginResponse::EditHitTarget(self.target.borrow().clone())
                }
                _ => PluginResponse::None,
            }
        }
    }

    fn app_with_mmap_cursor_plugin(
        target: Option<EditHitTarget>,
    ) -> (App, Rc<RefCell<Option<EditHitTarget>>>) {
        let target = Rc::new(RefCell::new(target));
        let mut app = App::new(None);
        let doc = DocumentView::new(vec!["abc".to_string()], 80, 10.0);
        app.push_entry_for_test(doc, Box::new(MmapCursorTestPlugin { target: target.clone() }));
        app.switch_workspace_for_test(0);
        (app, target)
    }

    fn last_cursor_icon(actions: &[AppAction]) -> CursorIcon {
        actions
            .iter()
            .rev()
            .find_map(|action| match action {
                AppAction::SetCursor(icon) => Some(*icon),
                _ => None,
            })
            .expect("cursor action should be emitted")
    }

    fn click_labeled_overlay_control(app: &mut App, label: &str) -> Vec<AppAction> {
        const LABEL_HIT_INSET_LOGICAL: f32 = 1.0;
        const LABEL_BASELINE_TO_CENTER_RATIO: f32 = 0.5;

        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let draw_list = app.ui_shell.paint_chrome(&app.current_theme, 1.0, Some(&mut shaper));
        let (x, y_baseline, font_size) = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                ui::core::paint::DrawCmd::TextLayout { layout, x, y_baseline, .. }
                    if layout.text == label =>
                {
                    Some((*x, *y_baseline, layout.font_size))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected labeled control {label:?} to be painted"));
        let px = x + LABEL_HIT_INSET_LOGICAL;
        let py = y_baseline - font_size * LABEL_BASELINE_TO_CENTER_RATIO;

        handle_mouse_input_left(app, ElementState::Pressed, px, py);
        handle_mouse_input_left(app, ElementState::Released, px, py)
    }

    fn prepare_textora_settings_overlay(app: &mut App) {
        let theme = ui::theme::test_theme();
        let mut measure = ui::core::NoopMeasure;
        app.ui_shell.mark_layout_initialized_for_test();
        let inputs = app.build_shell_inputs();
        app.ui_shell.update_frame(
            ui::core::Screen::new(1_200.0, 800.0),
            &theme,
            &mut measure,
            &inputs,
        );
        app.ui_shell.push_overlay_with_policy(
            Box::new(ui::modal_frame::ModalFrame::new(
                "设置",
                Box::new(TextoraSettingsOverlay::new(
                    ui::settings_view::SettingsViewInput {
                        theme_mode: ui::ThemeMode::System,
                        font_family: "Menlo".to_owned(),
                        font_size: 15.0,
                        line_height_ratio: 1.618,
                        word_wrap: true,
                        markdown_first_line_indent: false,
                        show_line_numbers: true,
                        tab_width: 4,
                        view_mode: ui::view_mode::ViewMode::Sidebar,
                        show_status_bar: false,
                        persistence: ui::settings_view::SettingsPersistenceView::Saved,
                    },
                    SyncSettingsInput::default(),
                )),
            )),
            ui::OverlayLayout::Fixed(ui::core::Rect::new(0.0, 0.0, 720.0, 560.0)),
            ui::OverlayInputPolicy::Modal,
            ui::DismissPolicy::ExplicitOnly,
        );
        app.ui_shell.update_frame(
            ui::core::Screen::new(1_200.0, 800.0),
            &theme,
            &mut measure,
            &inputs,
        );
    }

    #[test]
    fn mindmap_style_title_action_translates_to_panel_toggle() {
        let mut actions = Vec::new();

        translate_title_bar_action(
            &ui::title_bar::TitleBarAction::ToggleMindmapStylePanel,
            &mut actions,
        );

        assert!(matches!(actions.as_slice(), [AppAction::ToggleMindmapStylePanel]));
    }

    #[test]
    fn mindmap_style_widget_actions_preserve_close_and_selected_theme() {
        let app = App::new(None);
        let fixtures = [
            ui::core::widget::MindmapStylePanelAction::Close,
            ui::core::widget::MindmapStylePanelAction::SelectTheme("tide".into()),
        ];

        for panel_action in fixtures {
            let widget_action = WidgetAction::MindmapStylePanel(panel_action.clone());
            let mut actions = Vec::new();

            translate_widget_action(&widget_action, &app, &mut actions);

            assert!(matches!(
                actions.as_slice(),
                [AppAction::MindmapStylePanel(mapped)] if mapped == &panel_action
            ));
        }
    }

    #[test]
    fn horizontal_canvas_thumb_drag() {
        let app = App::new(None);
        let widget_action =
            WidgetAction::CanvasScrollbars(ui::canvas_scrollbars::CanvasScrollbarsAction {
                axis: ui::canvas::CanvasAxis::Horizontal,
                action: ui::scrollbar::ScrollbarAction::DragTo(320.0),
            });
        let mut actions = Vec::new();

        translate_widget_action(&widget_action, &app, &mut actions);

        assert!(matches!(
            actions.as_slice(),
            [AppAction::CanvasScrollbar {
                axis: ui::canvas::CanvasAxis::Horizontal,
                action: ui::scrollbar::ScrollbarAction::DragTo(position),
            }] if *position == 320.0
        ));
    }

    /// 初始化 sidebar 模式的 widget 系统（Dock children + layout），
    /// 使 TitleBarWidget 参与 hit-test。
    fn init_sidebar_widgets(app: &mut App) {
        use crate::ui_shell::ShellInputs;

        use ui::core::{NoopMeasure, Screen};
        let theme = ui::theme::test_theme();
        let mut m = NoopMeasure;
        app.ui_shell.mark_layout_initialized_for_test();
        app.ui_shell.rebuild_and_layout(
            Screen::new(1200.0, 800.0),
            &theme,
            &mut m,
            &ShellInputs {
                tabs_visible: false,
                tabs_thickness: 0.0,
                search_visible: false,
                search_thickness: 0.0,
                status_thickness: 24.0,
                sidebar_visible: true,
                sidebar_thickness: 220.0,
                scrollbar_thickness: 12.0,
                toc_visible: false,
                toc_thickness: 0.0,
                metrics: ui::settings::UiMetrics::from_settings(
                    &ui::settings::Settings::new(),
                    1.0,
                ),
                sidebar_settings: Default::default(),
            },
        );
    }

    #[test]
    fn test_handle_cursor_moved_sets_text_cursor() {
        let mut app = App::new(None);
        let actions = handle_cursor_moved(&mut app, 300.0, 100.0);
        let has_text_cursor =
            actions.iter().any(|a| matches!(a, AppAction::SetCursor(CursorIcon::Text)));
        assert!(has_text_cursor, "Mouse hover over editor should set the Text cursor");
    }

    #[test]
    fn pointer_leave_clears_hover_and_cursor_without_ending_editor_capture() {
        let mut app = App::new(None);
        app.mouse.is_down = true;
        app.mouse.last_hover_tab = Some(0);
        app.mouse.last_hover_redraw_pos = Some((10.0, 20.0));

        let actions = handle_pointer_leave(&mut app);

        assert!(app.mouse.is_down, "leaving the window must preserve legal pointer capture");
        assert_eq!(app.mouse.last_hover_tab, None);
        assert_eq!(app.mouse.last_hover_redraw_pos, None);
        assert!(actions.iter().any(|action| matches!(action, AppAction::HoverTab(None))));
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, AppAction::SetCursor(CursorIcon::Default)))
        );
        assert!(actions.iter().any(|action| matches!(action, AppAction::RequestRedraw)));
    }

    #[test]
    fn mmap_cursor_reflects_edit_and_drag_targets() {
        let (mut app, target) = app_with_mmap_cursor_plugin(Some(EditHitTarget::TextCaret {
            byte_offset: 1,
            selection_scope: Some(0..3),
        }));
        let pointer = (300.0, 100.0);

        assert_eq!(
            last_cursor_icon(&handle_cursor_moved(&mut app, pointer.0, pointer.1)),
            CursorIcon::Text
        );

        *target.borrow_mut() = Some(EditHitTarget::SourceObject { source_range: 1..2 });
        assert_eq!(
            last_cursor_icon(&handle_cursor_moved(&mut app, pointer.0, pointer.1)),
            CursorIcon::Grab
        );

        let source_generation = {
            let Some(entry) = app.active_tab_session_mut() else {
                panic!("mmap cursor test requires an active entry");
            };
            entry.document.cursor_move_to_offset(2);
            entry.document.cursor_mut().selection_anchor = Some(1);
            entry.document.generation()
        };
        assert_eq!(
            last_cursor_icon(&handle_cursor_moved(&mut app, pointer.0, pointer.1)),
            CursorIcon::Grab
        );

        app.mouse.canvas_drag = Some(crate::mouse::CanvasDragSession {
            source_range: 1..2,
            pressed_at: pointer,
            source_generation,
            eligibility: crate::mouse::CanvasDragEligibility::Enabled,
            started: true,
        });
        assert_eq!(
            last_cursor_icon(&handle_cursor_moved(&mut app, pointer.0, pointer.1)),
            CursorIcon::Grabbing
        );
        app.mouse.canvas_drag = None;

        *target.borrow_mut() = Some(EditHitTarget::ClearFocus);
        assert_eq!(
            last_cursor_icon(&handle_cursor_moved(&mut app, pointer.0, pointer.1)),
            CursorIcon::Default
        );
    }

    #[test]
    fn test_title_bar_area_consumes_cursor_in_sidebar_mode() {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        init_sidebar_widgets(&mut app);
        let title_h = ui::title_bar::title_bar_height(app.ui_metrics().dpi);
        let actions = handle_cursor_moved(&mut app, 300.0, title_h * 0.5);
        let has_text_cursor =
            actions.iter().any(|a| matches!(a, AppAction::SetCursor(CursorIcon::Text)));
        assert!(!has_text_cursor, "Cursor in title bar area should not reach editor");
    }

    #[test]
    fn test_cursor_below_title_bar_reaches_editor_in_sidebar_mode() {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        init_sidebar_widgets(&mut app);
        let title_h = ui::title_bar::title_bar_height(app.ui_metrics().dpi);
        let actions = handle_cursor_moved(&mut app, 300.0, title_h + 10.0);
        let has_text_cursor =
            actions.iter().any(|a| matches!(a, AppAction::SetCursor(CursorIcon::Text)));
        assert!(has_text_cursor, "Cursor below title bar should reach editor");
    }

    #[test]
    fn test_title_bar_area_consumes_mouse_input_in_sidebar_mode() {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        init_sidebar_widgets(&mut app);
        let title_h = ui::title_bar::title_bar_height(app.ui_metrics().dpi);
        let actions =
            handle_mouse_input_left(&mut app, ElementState::Pressed, 300.0, title_h * 0.5);
        let has_editor_input =
            actions.iter().any(|a| matches!(a, AppAction::EditorMouseInput { .. }));
        assert!(
            !has_editor_input,
            "Mouse press in title bar should not reach editor in sidebar mode"
        );
    }

    #[test]
    fn test_editor_mouseup_not_consumed_during_drag_in_sidebar_mode() {
        // Bug: TitleBarWidget unconditionally consumes MouseUp, which would
        // prevent the editor from receiving it and leave is_down stuck at true.
        // After the fix, MouseUp falls through when the editor is dragging.
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        init_sidebar_widgets(&mut app);

        let title_h = ui::title_bar::title_bar_height(app.ui_metrics().dpi);

        // ── 场景 1: 编辑器拖拽中 → MouseUp 必须穿透到 editor ──
        app.mouse.is_down = true;
        let actions =
            handle_mouse_input_left(&mut app, ElementState::Released, 300.0, title_h + 20.0);
        assert!(
            actions.iter().any(|a| matches!(
                a,
                AppAction::EditorMouseInput { state: ElementState::Released, .. }
            )),
            "拖拽中的 MouseUp 必须穿透 widget 到达 editor"
        );

        // ── 场景 2: 非编辑器发起的点击 → widget 消费 MouseUp 正常 ──
        let mut app2 = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        init_sidebar_widgets(&mut app2);
        // is_down = false 表示点击被 widget 处理了（如点击在 title bar 上）
        app2.mouse.is_down = false;
        let actions =
            handle_mouse_input_left(&mut app2, ElementState::Released, 300.0, title_h + 20.0);
        assert!(
            !actions.iter().any(|a| matches!(
                a,
                AppAction::EditorMouseInput { state: ElementState::Released, .. }
            )),
            "非编辑器发起的 MouseUp 不应穿透到 editor"
        );

        // ── 场景 3: 完整点击流程 ──
        let mut app3 = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        init_sidebar_widgets(&mut app3);
        // MouseDown 到达 editor
        let actions =
            handle_mouse_input_left(&mut app3, ElementState::Pressed, 300.0, title_h + 20.0);
        assert!(
            actions.iter().any(|a| matches!(
                a,
                AppAction::EditorMouseInput { state: ElementState::Pressed, .. }
            )),
            "编辑器区域 MouseDown 应到达 editor"
        );
        // 模拟 dispatch 后的状态
        app3.mouse.is_down = true;
        // MouseUp 也必须到达 editor（不能被子 widget 截断）
        let actions =
            handle_mouse_input_left(&mut app3, ElementState::Released, 300.0, title_h + 20.0);
        assert!(
            actions.iter().any(|a| matches!(
                a,
                AppAction::EditorMouseInput { state: ElementState::Released, .. }
            )),
            "编辑器区域 MouseUp 应到达 editor，不被 TitleBarWidget 截断"
        );
    }

    #[test]
    fn test_mouseup_not_consumed_by_widgets_in_tabs_mode() {
        // Tabs 模式下没有 TitleBarWidget，MouseUp 不受影响。
        // 此测试确保修复没有引入 Tabs 模式的回归。
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;

        // 拖拽中
        app.mouse.is_down = true;
        let actions = handle_mouse_input_left(&mut app, ElementState::Released, 300.0, 50.0);
        assert!(
            actions.iter().any(|a| matches!(
                a,
                AppAction::EditorMouseInput { state: ElementState::Released, .. }
            )),
            "Tabs 模式拖拽中 MouseUp 应到达 editor"
        );

        // 非拖拽中
        let mut app2 = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
        app2.mouse.is_down = false;
        let actions = handle_mouse_input_left(&mut app2, ElementState::Released, 300.0, 50.0);
        assert!(
            actions.iter().any(|a| matches!(
                a,
                AppAction::EditorMouseInput { state: ElementState::Released, .. }
            )),
            "Tabs 模式非拖拽中 MouseUp 也应到达 editor（无 widget 消费）"
        );
    }

    #[test]
    fn test_title_bar_area_consumes_right_click_in_sidebar_mode() {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        init_sidebar_widgets(&mut app);
        let title_h = ui::title_bar::title_bar_height(app.ui_metrics().dpi);
        let actions =
            handle_mouse_input_right(&mut app, ElementState::Pressed, 300.0, title_h * 0.5);
        // Title bar widget consumes the event; only SetCursor should be present
        let has_editor_input =
            actions.iter().any(|a| matches!(a, AppAction::EditorMouseInput { .. }));
        assert!(!has_editor_input, "Right-click in title bar area should not reach editor");
    }

    #[test]
    fn test_tabs_mode_no_title_bar_interception() {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
        let actions = handle_cursor_moved(&mut app, 300.0, 5.0);
        let has_text_cursor =
            actions.iter().any(|a| matches!(a, AppAction::SetCursor(CursorIcon::Text)));
        assert!(has_text_cursor, "Tabs mode should not intercept cursor near top");
    }
    #[test]
    fn translate_sidebar_settings_action_maps_only_to_sidebar_settings_menu() {
        let sa = ui::sidebar::SidebarAction::OpenSettingsMenu;
        let mut actions: Vec<AppAction> = Vec::new();
        let app = App::new(None);
        translate_sidebar_action(&app, &sa, &mut actions);
        assert!(
            matches!(actions.as_slice(), [AppAction::OpenSidebarSettingsMenu]),
            "OpenSettingsMenu should translate only to OpenSidebarSettingsMenu"
        );
    }

    #[test]
    fn widget_action_has_no_standalone_sync_variant() {
        let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/src/core/widget.rs"));

        assert!(!source.contains(concat!("Sync", "Panel(")));
        assert!(!source.contains(concat!("SYNC", "_PANEL")));
    }

    #[test]
    fn test_translate_sidebar_action_toggle_pin_maps_to_toggle_sidebar_pin() {
        // Bug A fix: SidebarAction::TogglePin must map to ToggleSidebarPin, NOT workspace TogglePin
        let sa = ui::sidebar::SidebarAction::TogglePin;
        let mut actions: Vec<AppAction> = Vec::new();
        let app = App::new(None);
        translate_sidebar_action(&app, &sa, &mut actions);
        assert!(!actions.is_empty(), "TogglePin must produce an AppAction");
        assert!(
            matches!(&actions[0], AppAction::ToggleSidebarPin),
            "TogglePin must translate to ToggleSidebarPin, not workspace TogglePin"
        );
    }

    #[test]
    fn test_translate_sidebar_action_open_document() {
        let sa = ui::sidebar::SidebarAction::OpenDocument;
        let mut actions: Vec<AppAction> = Vec::new();
        let app = App::new(None);
        translate_sidebar_action(&app, &sa, &mut actions);
        assert!(!actions.is_empty(), "OpenDocument must produce an AppAction");
        assert!(
            matches!(&actions[0], AppAction::ExecuteAppCommands(cmds) if cmds.len() == 1),
            "OpenDocument must translate to ExecuteAppCommands with one command"
        );
    }

    #[test]
    fn translate_sidebar_new_document_preserves_kind() {
        let sidebar_action =
            ui::sidebar::SidebarAction::NewDocument(ui::sidebar::NewDocumentKind::Mindmap);
        let mut actions = Vec::new();
        let app = App::new(None);

        translate_sidebar_action(&app, &sidebar_action, &mut actions);

        assert!(matches!(
            actions.as_slice(),
            [AppAction::NewDocument(ui::sidebar::NewDocumentKind::Mindmap)]
        ));
    }

    #[test]
    fn test_translate_tab_action_hover_tab_some() {
        let mut app = App::new(None);
        for _ in 0..4 {
            open_untitled_fixture(&mut app);
        }
        let expected_id = app.editor_tab_id_at(3).unwrap();
        let mut actions: Vec<AppAction> = Vec::new();
        let ta = ui::tab_bar::TabBarAction::HoverTab(Some(3));
        translate_tab_action(&ta, &app, &mut actions);
        assert_eq!(actions.len(), 1, "HoverTab-Some-3 should produce exactly one action");
        assert!(
            matches!(&actions[0], AppAction::HoverTab(Some(id)) if *id == expected_id),
            "HoverTab-Some-3 should map to the stable tab ID"
        );
    }

    #[test]
    fn test_translate_tab_action_hover_tab_none() {
        let app = App::new(None);
        let mut actions: Vec<AppAction> = Vec::new();
        let ta = ui::tab_bar::TabBarAction::HoverTab(None);
        translate_tab_action(&ta, &app, &mut actions);
        assert_eq!(actions.len(), 1, "HoverTab with None should produce exactly one action");
        assert!(
            matches!(&actions[0], AppAction::HoverTab(None)),
            "HoverTab-None should map to correct action"
        );
    }

    #[test]
    fn popup_context_action_keeps_original_tab_after_reorder() {
        let mut app = App::new(None);
        for _ in 0..3 {
            open_untitled_fixture(&mut app);
        }

        let closed_id = app.editor_tab_id_at(0).expect("tab 0 must exist");
        let original_target_id = app.editor_tab_id_at(1).expect("tab 1 must exist");
        let popup_action = ui::popup_menu::PopupMenuAction::Context {
            action: ui::popup_menu::ContextMenuAction::Close,
            tab_index: 1,
        };
        let popup_menu =
            ui::popup_menu::PopupMenu::context_px(1, (240.0, 180.0), (1200.0, 800.0), false, 1.0);

        let _ = app.dispatch_chrome_action(
            crate::dispatch::chrome::ChromeDispatchAction::OpenPopup(popup_menu),
        );

        let effect =
            app.close_editor_tab(closed_id).expect("closing tab 0 should reorder remaining tabs");
        assert_eq!(
            effect,
            crate::workspace::WorkspaceEffect::Closed { closed: closed_id, activated: None }
        );
        assert_eq!(
            app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
            app.editor_runtime_tab_ids()
        );

        let mut actions = Vec::new();
        translate_popup_action(&popup_action, &app, &mut actions);

        assert!(
            matches!(
                actions.as_slice(),
                [AppAction::ExecuteContextMenuAction(ui::popup_menu::ContextMenuAction::Close, id)]
                    if *id == original_target_id
            ),
            "popup action should still target the tab that was at index 1 when the menu opened"
        );
    }

    #[test]
    fn test_overlay_action_consumes_mouse_input_without_editor_fallthrough() {
        let mut app = App::new(None);
        let overlay_rect = ui::core::geom::Rect::new(0.0, 0.0, 1200.0, 800.0);
        let overlay_widget = Box::new(FakeOverlay {
            hit_all: true,
            action: Some(ui::core::widget::WidgetAction::Overlay(
                ui::core::overlay::OverlayAction::DismissRequested,
            )),
        });
        app.ui_shell.push_overlay(overlay_widget, overlay_rect);

        let actions = handle_mouse_input_left(&mut app, ElementState::Pressed, 300.0, 100.0);

        assert!(
            !actions.iter().any(|action| matches!(action, AppAction::EditorMouseInput { .. })),
            "Overlay action must consume mouse input without falling through to editor"
        );
    }

    #[test]
    fn overlay_dismiss_action_maps_once_and_is_consumed() {
        let app = App::new(None);
        let widget_action =
            WidgetAction::Overlay(ui::core::overlay::OverlayAction::DismissRequested);
        let mut actions = Vec::new();

        translate_widget_action(&widget_action, &app, &mut actions);

        assert!(matches!(actions.as_slice(), [AppAction::DismissOverlay]));
        assert!(widget_action_consumes_editor_fallthrough(&widget_action));
    }

    #[test]
    fn every_settings_view_action_translates_once_and_consumes_input() {
        let app = App::new(None);
        let fixtures = vec![
            ui::settings_view::SettingsViewAction::SetThemeMode(ui::ThemeMode::Dark),
            ui::settings_view::SettingsViewAction::SetFontFamily("Iosevka".into()),
            ui::settings_view::SettingsViewAction::SetFontSize(18.0),
            ui::settings_view::SettingsViewAction::SetLineHeightRatio(1.5),
            ui::settings_view::SettingsViewAction::SetWordWrap(false),
            ui::settings_view::SettingsViewAction::SetMarkdownFirstLineIndent(true),
            ui::settings_view::SettingsViewAction::SetShowLineNumbers(false),
            ui::settings_view::SettingsViewAction::SetTabWidth(8),
            ui::settings_view::SettingsViewAction::SetViewMode(ui::view_mode::ViewMode::Tabs),
            ui::settings_view::SettingsViewAction::SetShowStatusBar(true),
            ui::settings_view::SettingsViewAction::RetryPersistence,
        ];

        for settings_action in fixtures {
            let widget_action = WidgetAction::Settings(settings_action.clone());
            let mut app_actions = Vec::new();
            translate_widget_action(&widget_action, &app, &mut app_actions);

            assert!(widget_action_consumes_editor_fallthrough(&widget_action));
            assert!(matches!(
                app_actions.as_slice(),
                [AppAction::Settings(mapped)] if mapped == &settings_action
            ));
        }
    }

    #[test]
    fn sync_settings_action_is_extracted_after_product_overlay_dispatch() {
        let mut app = App::new(None);
        prepare_textora_settings_overlay(&mut app);

        let category_actions = click_labeled_overlay_control(&mut app, "同步");
        assert!(category_actions.is_empty());

        let actions = click_labeled_overlay_control(&mut app, "保存连接");
        assert!(matches!(
            actions.as_slice(),
            [AppAction::Sync(
                crate::sync_settings_types::SyncSettingsAction::ConfigureConnection { .. }
            )]
        ));
        assert_eq!(app.take_pending_sync_settings_action(), None);
    }

    #[test]
    fn test_overlay_consumes_mouse_move_sidebar_does_not_receive() {
        // When an overlay covers the sidebar area, sidebar should not receive MouseMove.
        // The overlay's dispatch returns None after consuming, so no sidebar action is produced.
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        init_sidebar_widgets(&mut app);

        // Push a fake overlay that covers the entire screen
        let overlay_rect = ui::core::geom::Rect::new(0.0, 0.0, 1200.0, 800.0);
        let overlay_widget = Box::new(FakeOverlay { hit_all: true, action: None });
        app.ui_shell.push_overlay(overlay_widget, overlay_rect);

        // Dispatch a MouseMove in the sidebar area
        let mut ctx = ui::core::widget::EventCtx::new(&app.current_theme, 1.0);
        let result = app
            .ui_shell
            .dispatch(&ui::core::widget::Event::MouseMove { px: 50.0, py: 400.0 }, &mut ctx);
        // Overlay consumed the event, so no sidebar action should be produced
        // (overlay returns None after consuming)
        assert!(result.is_none(), "Overlay should consume MouseMove, leaving nothing for sidebar");
    }

    /// Stub overlay widget that always hits and consumes events.
    struct FakeOverlay {
        hit_all: bool,
        action: Option<ui::core::widget::WidgetAction>,
    }
    impl ui::core::widget::Widget for FakeOverlay {
        fn set_rect(
            &mut self,
            _rect: ui::core::geom::Rect,
            _ctx: &mut ui::core::widget::LayoutCtx,
        ) {
        }
        fn paint(&self, _ctx: &mut ui::core::widget::PaintCtx) {}
        fn hit(&self, _px: f32, _py: f32) -> bool {
            self.hit_all
        }
        fn on_event(
            &mut self,
            _ev: &ui::core::widget::Event,
            _ctx: &mut ui::core::widget::EventCtx,
        ) -> Option<ui::core::widget::WidgetAction> {
            self.action.clone()
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }
    #[test]
    fn test_sidebar_click_produces_switch_tab() {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;

        // The workspace must contain tabs that correspond to the sidebar items so
        // that the sidebar index can be resolved to a stable TabId.
        open_untitled_fixture(&mut app);
        open_untitled_fixture(&mut app);

        // Inject sidebar tabs data before building widgets
        let dpi = app.ui_metrics().dpi;
        let tabs = vec![
            ui::tab_bar::TabInfo {
                title: "a.rs".into(),
                file_path: None,
                is_dirty: false,
                pinned: false,
                language: String::new(),
            },
            ui::tab_bar::TabInfo {
                title: "b.rs".into(),
                file_path: None,
                is_dirty: false,
                pinned: false,
                language: String::new(),
            },
        ];
        app.ui_shell.set_sidebar_input(
            ui::sidebar::SidebarConfig::new_default(dpi),
            tabs,
            Some(0),
            (68.0 * dpi, 0.0),
        );

        init_sidebar_widgets(&mut app);

        // Click on the second list item (b.rs).
        // list_clip starts at y=137.0, row_h=28, so item 1 is at y=165.0..193.0
        // Click at y=180 (within item 1 range)
        let pressed_actions =
            handle_mouse_input_left(&mut app, ElementState::Pressed, 110.0, 180.0);
        assert!(
            pressed_actions.iter().any(|action| matches!(action, AppAction::SwitchTab(_))),
            "Sidebar document selection must switch on mouse press, matching the tab bar"
        );

        let actions = handle_mouse_input_left(&mut app, ElementState::Released, 110.0, 180.0);

        assert!(
            !actions.iter().any(|action| matches!(action, AppAction::SwitchTab(_))),
            "Mouse release must not emit a duplicate sidebar tab switch"
        );
    }

    #[test]
    fn left_click_editor_clears_search_keyboard_focus() {
        let mut app = App::new(None);
        app.ui_shell.focus_widget(ui::core::widget::ids::SEARCH_BAR);

        // Simulating click in editor
        let _ = handle_mouse_input_left(&mut app, ElementState::Pressed, 100.0, 100.0);

        assert_eq!(app.ui_shell.keyboard_focus(), crate::ui_shell::KeyboardFocusTarget::Editor);
    }

    #[test]
    fn left_click_editor_clears_mindmap_style_keyboard_focus() {
        let mut app = App::new(None);
        app.ui_shell.focus_widget(ui::core::widget::ids::MINDMAP_STYLE_PANEL);

        let _ = handle_mouse_input_left(&mut app, ElementState::Pressed, 100.0, 100.0);

        assert_eq!(app.ui_shell.keyboard_focus(), crate::ui_shell::KeyboardFocusTarget::Editor);
    }

    #[test]
    fn left_click_consumed_by_mindmap_style_panel_keeps_panel_focus() {
        let mut app = App::new(None);
        let theme = ui::theme::test_theme();
        let mut measure = ui::core::NoopMeasure;
        app.ui_shell.mark_layout_initialized_for_test();
        app.ui_shell.set_mindmap_style_panel_input(
            Some(ui::mindmap_style_panel::MindmapStylePanelInput::from_selection(
                ui::theme::MindmapThemeSelection::Default,
                true,
            )),
            1.0,
        );
        let inputs = app.build_shell_inputs();
        app.ui_shell.update_frame(
            ui::core::Screen::new(1_200.0, 800.0),
            &theme,
            &mut measure,
            &inputs,
        );
        app.ui_shell.focus_widget(ui::core::widget::ids::MINDMAP_STYLE_PANEL);
        let panel_x = 1_200.0 - ui::mindmap_style_panel::PANEL_WIDTH_LOGICAL + 8.0;

        let actions = handle_mouse_input_left(&mut app, ElementState::Pressed, panel_x, 400.0);

        assert_eq!(
            app.ui_shell.keyboard_focus(),
            crate::ui_shell::KeyboardFocusTarget::Widget(
                ui::core::widget::ids::MINDMAP_STYLE_PANEL
            )
        );
        assert!(!actions.iter().any(|action| matches!(action, AppAction::EditorMouseInput { .. })));
    }

    #[test]
    fn left_click_non_editor_chrome_does_not_clear_search_keyboard_focus() {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        init_sidebar_widgets(&mut app);
        app.ui_shell.focus_widget(ui::core::widget::ids::SEARCH_BAR);

        // Simulating click on titlebar area
        let title_h = ui::title_bar::title_bar_height(app.ui_metrics().dpi);
        let _ = handle_mouse_input_left(&mut app, ElementState::Pressed, 300.0, title_h * 0.5);

        assert_eq!(
            app.ui_shell.keyboard_focus(),
            crate::ui_shell::KeyboardFocusTarget::Widget(ui::core::widget::ids::SEARCH_BAR)
        );
    }

    #[test]
    fn clearing_search_query_does_not_clear_keyboard_focus() {
        let mut app = App::new(None);
        app.ui_shell.focus_widget(ui::core::widget::ids::SEARCH_BAR);

        // Simulate search action that clears query. The focus should remain.
        let action = ui::search_bar::SearchBarAction::QueryChanged(String::new());
        let _effect = app.dispatch_search_action(action);

        assert_eq!(
            app.ui_shell.keyboard_focus(),
            crate::ui_shell::KeyboardFocusTarget::Widget(ui::core::widget::ids::SEARCH_BAR)
        );
    }
}
