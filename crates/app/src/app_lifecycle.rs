use crate::App;
/// Application state and winit event handling.
use crate::app_event::AppEvent;

use crate::app::compute_cursor_phase;

use appkit_shell::ProductHost;
use appkit_shell::accessibility_adapter::{
    PlatformAccessibilityEvent, PlatformAccessibilityWindowEvent,
};
use appkit_shell::editor_runtime::{EditorFocus, EditorInputContext};
pub(crate) use appkit_shell::window_input::winit_key_to_keycode;
use appkit_shell::window_input::{scroll_delta_pixels, ui_modifiers};
use std::path::{Path, PathBuf};
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{Ime, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::WindowId;

use crate::actions::AppAction;
use crate::file_history::compute_workspace_root;
use crate::input::{EditCommand, key_to_command};
use crate::native_menu::{MenuAction, NativeMenu};
use crate::textora_product::ProductEventSender;
use ui::core::overlay::OverlayAction;
use ui::core::widget::{KeyCode, Modifiers, WidgetAction};
#[allow(unused_imports)]
use ui::render_geom::{AdvanceCacheEntry, compute_selection_highlight_quads};

// Font size and line height are now in Settings
const FALLBACK_LOCAL_DEVICE_SHORT_ID: &str = "local";
const ACCESSIBILITY_WINDOW_NAME: &str = "textora";

fn editor_input_context(app: &App) -> EditorInputContext {
    let editor_focus =
        matches!(app.ui_shell.keyboard_focus(), crate::ui_shell::KeyboardFocusTarget::Editor)
            && app.editor_runtime.window_focused();
    EditorInputContext {
        editor_rect: app.ui_shell.editor_rect(),
        focus: if editor_focus { EditorFocus::Active } else { EditorFocus::Inactive },
        modal_blocked: app.ui_shell.active_overlay_is_modal(),
    }
}

/// 计算光标当前是否可见，以及下一次切换的时间点。

// ── Search bar keyboard routing helpers ──

/// Returns true if the key combo should bypass search bar forwarding
/// and go through the normal key_to_command → handle_command pipeline.
fn is_search_bar_whitelist(
    logical_key: &winit::keyboard::Key,
    sup: bool,
    shift: bool,
    alt: bool,
) -> bool {
    if !sup {
        return false; // Only Cmd-key combos are whitelisted
    }
    match logical_key {
        Key::Character(c) => match c.as_str() {
            "f" if !shift && !alt => true, // Cmd+F
            "f" if shift && !alt => true,  // Cmd+Shift+F
            "s" if !shift && !alt => true, // Cmd+S
            "s" if shift && !alt => true,  // Cmd+Shift+S
            "w" if !shift && !alt => true, // Cmd+W
            "z" if !shift && !alt => true, // Cmd+Z
            "z" if shift && !alt => true,  // Cmd+Shift+Z
            "[" if !shift && !alt => true, // Cmd+[
            "]" if !shift && !alt => true, // Cmd+]
            "[" if shift && !alt => true,  // Cmd+Shift+[
            "]" if shift && !alt => true,  // Cmd+Shift+]
            _ => false,
        },
        Key::Named(NamedKey::ArrowLeft) if sup && alt && !shift => true, // Cmd+Alt+Left
        Key::Named(NamedKey::ArrowRight) if sup && alt && !shift => true, // Cmd+Alt+Right
        _ => false,
    }
}

fn load_recent_file_paths(
    file_history: &crate::file_history::FileHistory,
    workspace_root: Option<&Path>,
) -> Vec<PathBuf> {
    let entries = match workspace_root {
        Some(root) => file_history.get_by_workspace(root, crate::file_history::MENU_LIMIT),
        None => file_history.get_valid_entries(crate::file_history::MENU_LIMIT),
    };
    entries.into_iter().map(|entry| entry.file_path.clone()).collect()
}

fn spawn_recent_file_loader(
    send_wake: impl FnOnce(AppEvent) -> bool + Send + 'static,
    product_event_sender: ProductEventSender,
    file_history: crate::file_history::FileHistory,
    workspace_root: Option<PathBuf>,
) {
    std::thread::spawn(move || {
        let recent_paths = load_recent_file_paths(&file_history, workspace_root.as_deref());
        if product_event_sender.send_recent_files_loaded(recent_paths).is_err() {
            eprintln!("[startup] recent file loader could not reach product inbox");
            return;
        }
        if !send_wake(AppEvent::ProductWake) {
            eprintln!("[startup] recent file loader could not reach event loop");
        }
    });
}

fn canvas_pinch_action(delta: f64, mouse_position: (f64, f64)) -> Option<AppAction> {
    if delta.is_nan() {
        return None;
    }

    Some(AppAction::CanvasPinch {
        delta,
        screen_anchor: ui::canvas::CanvasPoint::new(
            mouse_position.0 as f32,
            mouse_position.1 as f32,
        ),
    })
}

enum ModalInputRoute {
    Dispatch(AppAction),
    Redraw,
}

enum FocusedWidgetInputRoute {
    Dispatch(AppAction),
    Redraw,
    Consumed,
}

fn modal_widget_action(action: &WidgetAction) -> Option<AppAction> {
    match action {
        WidgetAction::Settings(settings_action) => {
            Some(AppAction::Settings(settings_action.clone()))
        }
        WidgetAction::Overlay(OverlayAction::DismissRequested) => Some(AppAction::DismissOverlay),
        _ => None,
    }
}

fn modal_input_route(action: &WidgetAction) -> ModalInputRoute {
    match modal_widget_action(action) {
        Some(app_action) => ModalInputRoute::Dispatch(app_action),
        None => ModalInputRoute::Redraw,
    }
}

fn route_modal_keyboard_input(
    ui_shell: &mut crate::ui_shell::UiShell,
    key_code: Option<KeyCode>,
    modifiers: Modifiers,
    theme: &ui::Theme,
    dpi: f32,
) -> Option<ModalInputRoute> {
    if !ui_shell.active_overlay_is_modal() {
        return None;
    }

    let Some(key_code) = key_code else {
        return Some(ModalInputRoute::Redraw);
    };

    ui_shell.forward_key(key_code, modifiers, theme, dpi).map(|action| modal_input_route(&action))
}

fn route_mindmap_style_panel_keyboard_input(
    ui_shell: &mut crate::ui_shell::UiShell,
    key_code: Option<KeyCode>,
    modifiers: Modifiers,
    theme: &ui::Theme,
    dpi: f32,
) -> Option<FocusedWidgetInputRoute> {
    if ui_shell.keyboard_focus()
        != crate::ui_shell::KeyboardFocusTarget::Widget(ui::core::widget::ids::MINDMAP_STYLE_PANEL)
    {
        return None;
    }

    let action =
        key_code.and_then(|key_code| ui_shell.forward_key(key_code, modifiers, theme, dpi));
    Some(match action {
        Some(WidgetAction::MindmapStylePanel(action)) => {
            FocusedWidgetInputRoute::Dispatch(AppAction::MindmapStylePanel(action))
        }
        Some(WidgetAction::Consumed) => FocusedWidgetInputRoute::Redraw,
        Some(_) => FocusedWidgetInputRoute::Redraw,
        None => FocusedWidgetInputRoute::Consumed,
    })
}

fn mindmap_style_panel_should_receive_keyboard(
    ui_shell: &crate::ui_shell::UiShell,
    logical_key: &Key,
    sup: bool,
    shift: bool,
    alt: bool,
    ctrl: bool,
) -> bool {
    if ui_shell.keyboard_focus()
        != crate::ui_shell::KeyboardFocusTarget::Widget(ui::core::widget::ids::MINDMAP_STYLE_PANEL)
    {
        return false;
    }

    // The global Cmd+Shift+P shortcut is handled explicitly by events::handle_keyboard.
    if sup && shift && logical_key.to_text() == Some("p") {
        return false;
    }

    // Plain keys and Shift-only navigation belong to the panel. For modifier
    // combinations, let any command understood by the normal editor pipeline
    // through; only unbound shortcuts remain consumed by the panel.
    if !(sup || ctrl || alt) {
        return true;
    }

    let mut modifiers = ModifiersState::empty();
    modifiers.set(ModifiersState::SUPER, sup);
    modifiers.set(ModifiersState::SHIFT, shift);
    modifiers.set(ModifiersState::ALT, alt);
    modifiers.set(ModifiersState::CONTROL, ctrl);
    key_to_command(logical_key, modifiers).is_none()
}

fn mindmap_style_panel_consumes_ime(
    ui_shell: &crate::ui_shell::UiShell,
    event: &ui::core::Event,
) -> bool {
    ui_shell.keyboard_focus()
        == crate::ui_shell::KeyboardFocusTarget::Widget(ui::core::widget::ids::MINDMAP_STYLE_PANEL)
        && matches!(event, ui::core::Event::ImePreedit { .. } | ui::core::Event::ImeCommit(_))
}

fn route_modal_ime_input(
    ui_shell: &mut crate::ui_shell::UiShell,
    event: ui::core::Event,
    theme: &ui::Theme,
    dpi: f32,
) -> Option<ModalInputRoute> {
    if !ui_shell.active_overlay_is_modal() {
        return None;
    }

    ui_shell.forward_ime(event, theme, dpi).map(|action| modal_input_route(&action))
}

fn modal_wheel_event(
    delta: &winit::event::MouseScrollDelta,
    mouse_position: (f64, f64),
    line_height: f32,
) -> ui::core::Event {
    let (dx, dy) = scroll_delta_pixels(delta, line_height);
    ui::core::Event::Wheel { dx, dy, px: mouse_position.0 as f32, py: mouse_position.1 as f32 }
}

fn route_modal_wheel_input(
    ui_shell: &mut crate::ui_shell::UiShell,
    event: ui::core::Event,
    theme: &ui::Theme,
    dpi: f32,
) -> Option<ModalInputRoute> {
    if !ui_shell.active_overlay_is_modal() {
        return None;
    }

    let mut ctx = ui::core::widget::EventCtx { theme, dpi, cursor_hint: None };
    ui_shell.dispatch(&event, &mut ctx).map(|action| modal_input_route(&action))
}

fn route_modal_wheel_delta(
    ui_shell: &mut crate::ui_shell::UiShell,
    delta: &winit::event::MouseScrollDelta,
    mouse_position: (f64, f64),
    line_height: f32,
    theme: &ui::Theme,
    dpi: f32,
) -> Option<ModalInputRoute> {
    route_modal_wheel_input(
        ui_shell,
        modal_wheel_event(delta, mouse_position, line_height),
        theme,
        dpi,
    )
}

impl App {
    fn open_external_content_tab(
        &mut self,
        path: &Path,
        content: &str,
    ) -> crate::app_effect::AppEffect {
        let effect = if let Some(tab_id) = self.editor_tab_id_for_path(path) {
            self.activate_editor_tab(tab_id).unwrap_or(crate::workspace::WorkspaceEffect::None)
        } else {
            let dimensions = self.viewport_dimensions(self.screen_height());
            let crate::workspace_tab_factory::ProductPreparedTab { prepared, suggested_file_name } =
                self.prepare_external_editor_content(path, content, dimensions);
            self.install_editor_tab(
                prepared,
                suggested_file_name,
                appkit_shell::editor_runtime::OpenDisposition::Persistent,
            )
        };
        self.apply_workspace_effect(effect)
    }

    /// Centralized handler for system scale-factor changes.
    /// Updates settings, rescales sidebar, invalidates caches, and re-derives
    /// the display map for the active document.
    pub(crate) fn handle_scale_factor_changed(&mut self, scale_factor: f64) {
        let old_metrics = self.ui_metrics();
        self.update_scale_factor(scale_factor);
        let new_metrics = self.ui_metrics();
        let old_dpi = old_metrics.dpi;
        let new_dpi = new_metrics.dpi;
        if old_dpi > 0.0 && (old_dpi - new_dpi).abs() > f32::EPSILON {
            let ratio = new_dpi / old_dpi;
            self.ui_shell.scale_sidebar_width(ratio);
            self.ui_shell.sidebar_clamp_width(new_dpi);
        }
        let tab_ids = self.editor_tab_ids_in_order();
        for tab_id in &tab_ids {
            if let Some(mut tab) = self.tab_session_mut(*tab_id) {
                tab.invalidate_render_cache_all();
            }
        }
        if let Some(mut tab) = self.active_tab_session_mut() {
            tab.clear_advance_cache();
        }
        self.editor_runtime.clear_frame_cluster_pool();
        if let Some(active_index) = self.active_editor_index() {
            self.init_display_map(active_index);
        }
        self.invalidate_reshape();
        self.needs_redraw = true;
    }

    fn handle_dropped_file(&mut self, path: &std::path::Path) {
        match self.open_file(path) {
            Ok(effect) => self.apply_effect(effect),
            Err(error) => eprintln!("Error opening dropped file: {error}"),
        }
    }

    fn handle_open_file_requests(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            match self.open_file(&path) {
                Ok(effect) => self.apply_effect(effect),
                Err(error) => eprintln!("[macos] could not open {}: {error}", path.display()),
            }
        }
    }

    fn handle_user_event(&mut self, event: AppEvent) -> bool {
        match event {
            AppEvent::StartBackgroundServices => {
                self.start_background_services();
                false
            }
            AppEvent::ReshapeResultsReady => {
                self.needs_redraw = true;
                false
            }
            AppEvent::ProductWake => {
                let open_document_paths = self.product.drain_open_documents();
                self.handle_open_file_requests(open_document_paths);
                let effect = ProductHost::drain_product_events(&mut self.product);
                self.apply_effect(effect);
                false
            }
            AppEvent::FileSafetyResultsReady => {
                self.drain_file_safety_results();
                let monitor_reported_change =
                    self.library_file_monitor.as_ref().is_some_and(|monitor| {
                        let mut reported = false;
                        while let Some(batch) = monitor.try_recv() {
                            reported |= !batch.paths.is_empty();
                        }
                        reported
                    });
                if monitor_reported_change {
                    self.editor_runtime.request_file_safety_check_now(Instant::now());
                }
                self.needs_redraw = true;
                false
            }
            AppEvent::SaveResultsReady => self.drain_save_results(),
            AppEvent::Accessibility(_) => false,
        }
    }

    fn publish_accessibility_tree(&mut self) {
        let tree = self.ui_shell.accessibility_tree(ACCESSIBILITY_WINDOW_NAME);
        if let Err(errors) = tree.validate() {
            eprintln!("[accessibility] invalid semantic tree: {errors:?}");
            return;
        }
        let Some(adapter) = self.accessibility_adapter.as_mut() else { return };
        adapter.update(&tree);
    }

    fn handle_accessibility_event(
        &mut self,
        event: &PlatformAccessibilityEvent,
        event_loop: &ActiveEventLoop,
    ) {
        let Some(window) = self.editor_runtime.window() else { return };
        if window.id() != event.window_id {
            return;
        }

        match &event.window_event {
            PlatformAccessibilityWindowEvent::InitialTreeRequested => {
                self.publish_accessibility_tree();
            }
            PlatformAccessibilityWindowEvent::ActionRequested(platform_request) => {
                let shared_request = self
                    .accessibility_adapter
                    .as_ref()
                    .and_then(|adapter| adapter.translate_action(platform_request));
                let Some(shared_request) = shared_request else { return };
                let Some(widget_action) =
                    self.ui_shell.dispatch_accessibility_action(&shared_request)
                else {
                    return;
                };

                let mut actions = Vec::new();
                crate::events::translate_widget_action(&widget_action, self, &mut actions);
                if let Some(sync_action) = self.take_pending_sync_settings_action() {
                    actions.push(AppAction::Sync(sync_action));
                }
                for action in actions {
                    self.dispatch(action, event_loop);
                }
                self.needs_redraw = true;
            }
            PlatformAccessibilityWindowEvent::AccessibilityDeactivated => {}
        }
    }

    /// Actual resumed logic, wrapped in catch_unwind by the trait method.
    fn do_resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.running = true;
        if self.editor_runtime.window().is_none()
            && let Err(_e) = self.init_window(event_loop)
        {
            event_loop.exit();
            return;
        }
        // Build native menu bar after window creation (NSApp is ready)
        if self.native_menu().is_none() {
            let summaries = self.editor_runtime.document_summaries();
            let paths: Vec<&std::path::Path> =
                summaries.iter().filter_map(|summary| summary.path.as_deref()).collect();
            let workspace_root = compute_workspace_root(&paths);
            self.set_native_menu(NativeMenu::build_loading());
            let Some(event_loop_proxy) = self.event_loop_proxy.clone() else {
                eprintln!("[startup] recent file loader unavailable: event loop proxy missing");
                return;
            };
            spawn_recent_file_loader(
                move |event| event_loop_proxy.send_event(event).is_ok(),
                self.product.event_sender(),
                self.file_history.clone(),
                workspace_root,
            );
        }
    }

    /// Actual window event logic, wrapped in catch_unwind by the trait method.
    fn do_window_event(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        if let Some(adapter) = self.accessibility_adapter.as_mut()
            && let Some(window) = self.editor_runtime.window()
        {
            adapter.process_window_event(window, &event);
        }

        // Poll native menu actions (non-blocking)
        let menu_actions: Vec<MenuAction> = if let Some(native_menu) = self.native_menu() {
            let mut actions = Vec::new();
            while let Some(action) = native_menu.poll_action() {
                actions.push(action);
            }
            actions
        } else {
            Vec::new()
        };
        for action in menu_actions {
            if action == MenuAction::Quit {
                self.quit_app(event_loop);
                return;
            }
            self.dispatch_menu_action(action, event_loop);
        }
        match event {
            WindowEvent::CloseRequested => {
                // If there are unsaved changes, try to save first
                let dirty = self.active_tab_session().is_some_and(|tab| tab.document.dirty);
                let file_backed =
                    self.active_tab_session().is_some_and(|tab| tab.document.file_path.is_some());
                if dirty && file_backed {
                    let Some(tab_id) = self.active_tab_id() else {
                        self.quit_app(event_loop);
                        return;
                    };
                    self.pending_quit_after_save = true;
                    if let Err(error) = self.submit_editor_save(tab_id, None) {
                        self.pending_quit_after_save = false;
                        eprintln!("auto-save on close failed: {error}");
                        self.quit_app(event_loop);
                    }
                    return;
                }
                // If no file path (new unsaved doc), just discard
                self.quit_app(event_loop);
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.editor_runtime.set_input_modifiers(modifiers.state());
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                let dpi = self.editor_runtime.scale_factor() as f32;
                if self.ui_shell.active_overlay_is_modal() {
                    let _ = self.ui_shell.forward_ime(
                        ui::core::Event::ImePreedit { text, cursor },
                        &self.current_theme,
                        dpi,
                    );
                    let _ = self.editor_runtime.update_preedit(
                        editor_input_context(self),
                        String::new(),
                        None,
                    );
                } else {
                    let panel_event = ui::core::Event::ImePreedit { text: text.clone(), cursor };
                    if mindmap_style_panel_consumes_ime(&self.ui_shell, &panel_event) {
                        let _ = self.editor_runtime.update_preedit(
                            editor_input_context(self),
                            String::new(),
                            None,
                        );
                        self.needs_redraw = true;
                        return;
                    }
                    let action = self.ui_shell.forward_ime(panel_event, &self.current_theme, dpi);
                    if action.is_none() {
                        let _ = self.editor_runtime.update_preedit(
                            editor_input_context(self),
                            text,
                            cursor,
                        );
                    } else {
                        let _ = self.editor_runtime.update_preedit(
                            editor_input_context(self),
                            String::new(),
                            None,
                        );
                    }
                }
                self.needs_redraw = true;
            }
            WindowEvent::Ime(Ime::Enabled) => {
                let dpi = self.editor_runtime.scale_factor() as f32;
                let action =
                    self.ui_shell.forward_ime(ui::core::Event::ImeEnable, &self.current_theme, dpi);
                if action.is_none() {
                    let _ = self.editor_runtime.update_preedit(
                        editor_input_context(self),
                        String::new(),
                        None,
                    );
                }
                self.needs_redraw = true;
            }
            WindowEvent::Ime(Ime::Disabled) => {
                let dpi = self.editor_runtime.scale_factor() as f32;
                let action = self.ui_shell.forward_ime(
                    ui::core::Event::ImeDisable,
                    &self.current_theme,
                    dpi,
                );
                if action.is_none() {
                    let _ = self.editor_runtime.update_preedit(
                        editor_input_context(self),
                        String::new(),
                        None,
                    );
                }
                self.needs_redraw = true;
            }

            WindowEvent::Ime(Ime::Commit(text)) => {
                let dpi = self.editor_runtime.scale_factor() as f32;
                if self.ui_shell.active_overlay_is_modal() {
                    if let Some(route) = route_modal_ime_input(
                        &mut self.ui_shell,
                        ui::core::Event::ImeCommit(text),
                        &self.current_theme,
                        dpi,
                    ) {
                        match route {
                            ModalInputRoute::Dispatch(action) => self.dispatch(action, event_loop),
                            ModalInputRoute::Redraw => self.needs_redraw = true,
                        }
                    }
                } else {
                    let panel_event = ui::core::Event::ImeCommit(text.clone());
                    if mindmap_style_panel_consumes_ime(&self.ui_shell, &panel_event) {
                        self.needs_redraw = true;
                        return;
                    }
                    let action = self.ui_shell.forward_ime(panel_event, &self.current_theme, dpi);
                    if let Some(action) = action {
                        if let WidgetAction::SearchBar(ref search_action) = action {
                            let effect = self.dispatch_search_action(search_action.clone());
                            self.apply_effect(effect);
                        } else {
                            self.needs_redraw = true;
                        }
                    } else {
                        let effect =
                            self.dispatch_edit_command(EditCommand::InsertText(text), event_loop);
                        self.apply_effect(effect);
                        // Clear IME composition state
                        let _ = self.editor_runtime.update_preedit(
                            editor_input_context(self),
                            String::new(),
                            None,
                        );
                        // Reset WYSIWYG preferred x so the next vertical move
                        // doesn't anchor to a stale position
                        self.editor_runtime.set_preferred_x(None);
                        self.editor_runtime.set_preferred_x(None);
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state.is_pressed() => {
                if self.ui_shell.active_overlay_is_modal() {
                    let key_code = winit_key_to_keycode(&event.logical_key, event.text.as_deref());
                    let input_modifiers = self.editor_runtime.input_modifiers();
                    let modifiers = ui_modifiers(input_modifiers);
                    let dpi = self.editor_runtime.scale_factor() as f32;
                    let Some(route) = route_modal_keyboard_input(
                        &mut self.ui_shell,
                        key_code,
                        modifiers,
                        &self.current_theme,
                        dpi,
                    ) else {
                        return;
                    };
                    match route {
                        ModalInputRoute::Dispatch(action) => self.dispatch(action, event_loop),
                        ModalInputRoute::Redraw => self.needs_redraw = true,
                    }
                    return;
                }

                let input_modifiers = self.editor_runtime.input_modifiers();
                let sup = input_modifiers.super_key();
                let shift = input_modifiers.shift_key();
                let alt = input_modifiers.alt_key();
                let ctrl = input_modifiers.control_key();
                if mindmap_style_panel_should_receive_keyboard(
                    &self.ui_shell,
                    &event.logical_key,
                    sup,
                    shift,
                    alt,
                    ctrl,
                ) {
                    let focused_widget_key_code =
                        winit_key_to_keycode(&event.logical_key, event.text.as_deref());
                    let focused_widget_modifiers = ui_modifiers(input_modifiers);
                    let dpi = self.editor_runtime.scale_factor() as f32;
                    if let Some(route) = route_mindmap_style_panel_keyboard_input(
                        &mut self.ui_shell,
                        focused_widget_key_code,
                        focused_widget_modifiers,
                        &self.current_theme,
                        dpi,
                    ) {
                        match route {
                            FocusedWidgetInputRoute::Dispatch(action) => {
                                self.dispatch(action, event_loop)
                            }
                            FocusedWidgetInputRoute::Redraw => self.needs_redraw = true,
                            FocusedWidgetInputRoute::Consumed => {}
                        }
                        return;
                    }
                }

                // ── Search bar keyboard focus routing ──
                // When search bar has keyboard focus, route keys directly to it.
                // Whitelisted shortcuts still go through key_to_command → handle_command.
                if self.ui_shell.search_bar_has_keyboard_focus()
                    && !is_search_bar_whitelist(&event.logical_key, sup, shift, alt)
                {
                    if let Some(kc) =
                        winit_key_to_keycode(&event.logical_key, event.text.as_deref())
                    {
                        let mods = ui_modifiers(input_modifiers);
                        let dpi = self.editor_runtime.scale_factor() as f32;
                        if let Some(action) =
                            self.ui_shell.forward_key(kc, mods, &self.current_theme, dpi)
                        {
                            if let WidgetAction::SearchBar(ref sa) = action {
                                let effect = self.dispatch_search_action(sa.clone());
                                self.apply_effect(effect);
                            } else {
                                self.needs_redraw = true;
                            }
                        }
                    }
                    return; // consumed by search bar
                }
                let actions = crate::events::handle_keyboard(self, &event);
                for action in actions {
                    self.dispatch(action, event_loop);
                }
            }
            WindowEvent::Focused(focused) => {
                let effect = self.handle_window_focus_changed(focused);
                self.apply_effect(effect);
            }
            WindowEvent::ThemeChanged(_system_theme) => {
                let mode = self.settings.theme_mode;
                if mode == ui::settings::ThemeMode::System {
                    self.rebuild_theme_state();
                    self.needs_redraw = true;
                    if let Some(w) = self.editor_runtime.window() {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::Resized(physical_size) => {
                if let appkit_shell::editor_runtime::ResizeOutcome::Applied {
                    width_changed,
                    height,
                    ..
                } = self.editor_runtime.request_resize(physical_size.width, physical_size.height)
                {
                    self.apply_resize_layout(height, width_changed);
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.handle_scale_factor_changed(scale_factor);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.ui_shell.active_overlay_is_modal() {
                    let line_height = self.ui_metrics().line_height;
                    if let Some(route) = route_modal_wheel_delta(
                        &mut self.ui_shell,
                        &delta,
                        self.mouse.pos,
                        line_height,
                        &self.current_theme,
                        self.editor_runtime.scale_factor() as f32,
                    ) {
                        match route {
                            ModalInputRoute::Dispatch(action) => self.dispatch(action, event_loop),
                            ModalInputRoute::Redraw => self.needs_redraw = true,
                        }
                        return;
                    }
                }
                let actions = crate::events::handle_scroll(self, delta);
                for action in actions {
                    self.dispatch(action, event_loop);
                }
            }
            WindowEvent::PinchGesture { delta, .. } => {
                if let Some(action) = canvas_pinch_action(delta, self.mouse.pos) {
                    self.dispatch(action, event_loop);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                for action in crate::events::handle_pointer_leave(self) {
                    self.dispatch(action, event_loop);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let actions =
                    crate::events::handle_cursor_moved(self, position.x as f32, position.y as f32);
                for action in actions {
                    self.dispatch(action, event_loop);
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                let actions = crate::events::handle_mouse_input_left(
                    self,
                    state,
                    self.mouse.pos.0 as f32,
                    self.mouse.pos.1 as f32,
                );
                for action in actions {
                    self.dispatch(action, event_loop);
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Right, .. } => {
                let px = self.mouse.pos.0 as f32;
                let py = self.mouse.pos.1 as f32;
                let actions = crate::events::handle_mouse_input_right(self, state, px, py);
                for action in actions {
                    self.dispatch(action, event_loop);
                }
            }

            WindowEvent::DroppedFile(path) => self.handle_dropped_file(&path),
            WindowEvent::RedrawRequested => {
                // 诊断：记录 RedrawRequested 到达时间
                let _rr_arrive = std::time::Instant::now();
                let _since_last_rr = self.editor_runtime.note_reshape_result_arrived(_rr_arrive);

                self.needs_redraw = false;
                self.flush_pending_resize();
                // Tick sidebar hover state machine each frame
                if matches!(self.settings.view_mode, ui::view_mode::ViewMode::Sidebar) {
                    let (visibility_changed, animating) = {
                        let shell = &mut self.ui_shell;
                        shell.sidebar_tick(std::time::Instant::now())
                    };
                    self.sidebar_animating = animating || visibility_changed;
                    if self.sidebar_animating {
                        self.needs_redraw = true;
                    }
                } else {
                    self.sidebar_animating = false;
                }
                let _rr_t0 = std::time::Instant::now();
                self.render();
                self.publish_accessibility_tree();
                let _rr_us = _rr_t0.elapsed().as_micros();
                // Debug-only: append frame timing to /tmp/perf.log
                #[cfg(debug_assertions)]
                {
                    let _ = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open("/tmp/perf.log")
                        .and_then(|mut f| {
                            use std::io::Write;
                            writeln!(
                                f,
                                "[rr] render={}us since_last={}us nr_after_render={}",
                                _rr_us, _since_last_rr, self.needs_redraw
                            )
                        });
                }
            }
            _ => {}
        }
        self.update_ime_cursor_area();
    }

    fn handle_window_focus_changed(&mut self, focused: bool) -> crate::app_effect::AppEffect {
        if focused {
            self.editor_runtime.set_window_focus(true);
            return crate::app_effect::AppEffect::REDRAW;
        }

        let effect = crate::events::handle_interaction_cancel(self);
        // 失焦时冻结光标闪烁，保持可见状态
        if let Some(mut tab) = self.active_tab_session_mut() {
            tab.cursor_render_state_mut().cursor_blink_instant = std::time::Instant::now();
        }
        effect
    }

    pub(crate) fn drain_file_safety_notices(
        &mut self,
    ) -> Vec<crate::file_safety::FileSafetyNotice> {
        std::mem::take(&mut self.file_safety_notices)
    }

    fn submit_file_safety_checks(&mut self) {
        let now = Instant::now();
        if !self.editor_runtime.file_safety_should_check(now) {
            return;
        }
        self.editor_runtime.schedule_file_safety_check(now);
        let local_device_short_id = self.local_device_short_id();
        let _ = self.editor_runtime.submit_file_safety_checks(&local_device_short_id);
    }

    fn local_device_short_id(&self) -> String {
        let Some(controller) = self.sync_controller() else {
            return FALLBACK_LOCAL_DEVICE_SHORT_ID.to_owned();
        };
        match &controller.snapshot().connection {
            crate::sync_controller::SyncConnectionState::Connected { instance } => {
                instance.device_id.as_str().chars().take(6).collect()
            }
            _ => FALLBACK_LOCAL_DEVICE_SHORT_ID.to_owned(),
        }
    }

    fn drain_file_safety_results(&mut self) {
        for observation in self.editor_runtime.drain_file_safety_observations() {
            self.apply_file_safety_outcome(
                observation.tab_id,
                observation.path,
                observation.dirty,
                observation.content_revision,
                observation.outcome,
            );
        }
    }

    fn drain_save_results(&mut self) -> bool {
        let mut should_quit = false;
        for completion in self.editor_runtime.drain_save_completions() {
            let outcome = self.editor_runtime.apply_save_completion(completion);
            let completed_tab = outcome.notifications.iter().find_map(|notification| {
                if let appkit_shell::editor_runtime::EditorNotification::SaveCompleted {
                    tab_id,
                    ..
                } = notification
                {
                    Some(*tab_id)
                } else {
                    None
                }
            });
            let failed_tab = outcome.notifications.iter().find_map(|notification| {
                if let appkit_shell::editor_runtime::EditorNotification::SaveFailed {
                    tab_id, ..
                } = notification
                {
                    Some(*tab_id)
                } else {
                    None
                }
            });
            for notification in &outcome.notifications {
                match notification {
                    appkit_shell::editor_runtime::EditorNotification::DirtyChanged {
                        tab_id,
                        dirty,
                    } if self.active_tab_id() == Some(*tab_id) => {
                        self.update_document_edited(*dirty);
                    }
                    appkit_shell::editor_runtime::EditorNotification::PathChanged {
                        tab_id,
                        ..
                    } => {
                        self.clear_editor_suggested_file_name(*tab_id);
                        self.refresh_file_monitor_roots();
                    }
                    appkit_shell::editor_runtime::EditorNotification::SaveFailed {
                        message,
                        ..
                    } => eprintln!("save error: {message}"),
                    _ => {}
                }
            }
            let mut effect = outcome.shell_effect;
            if let Some(tab_id) = failed_tab {
                self.pending_close_after_save.remove(&tab_id);
                self.pending_quit_after_save = false;
            }
            if let Some(tab_id) = completed_tab
                && self.pending_close_after_save.remove(&tab_id)
                && self
                    .editor_runtime
                    .document_summary(tab_id)
                    .is_some_and(|summary| !summary.dirty)
                && let Some(index) = self.editor_tab_index(tab_id)
            {
                self.record_entry_to_history(index);
                if let Some(workspace_effect) = self.close_editor_tab(tab_id) {
                    effect = effect.merge(self.apply_workspace_effect(workspace_effect));
                }
                self.save_history();
                self.rebuild_native_menu();
            }
            if self.pending_quit_after_save
                && completed_tab.is_some_and(|tab_id| self.active_tab_id() == Some(tab_id))
                && self.active_tab_session().is_none_or(|tab| !tab.document.dirty)
            {
                self.pending_quit_after_save = false;
                should_quit = true;
            }
            self.apply_effect(effect);
        }
        should_quit
    }

    fn apply_file_safety_outcome(
        &mut self,
        tab_id: appkit_core::workspace::types::TabId,
        path: std::path::PathBuf,
        expected_dirty: bool,
        expected_content_revision: u64,
        outcome: Result<crate::file_safety::FileSafetyOutcome, crate::file_safety::FileSafetyError>,
    ) {
        let Some(tab) = self.tab_session(tab_id) else {
            self.editor_runtime.forget_file_safety_tab(tab_id);
            return;
        };
        let is_current = tab.document.dirty == expected_dirty
            && tab.document.file_path.as_ref() == Some(&path)
            && tab.document.content_revision() == expected_content_revision;
        if !is_current {
            if let Ok(crate::file_safety::FileSafetyOutcome::Conflict { conflict, .. }) = outcome {
                self.file_safety_notices.push(
                    crate::file_safety::FileSafetyNotice::ConflictCopyCreated {
                        original: path.clone(),
                        conflict,
                    },
                );
            }
            self.editor_runtime.forget_file_safety_tab(tab_id);
            self.editor_runtime.request_file_safety_check_now(Instant::now());
            return;
        }
        match outcome {
            Ok(crate::file_safety::FileSafetyOutcome::Unchanged) => {}
            Ok(crate::file_safety::FileSafetyOutcome::Reload { content, .. }) => {
                if !expected_dirty && tab.document.full_text() == content {
                    return;
                }
                let cursor_offset = tab.document.cursor_offset().to_usize();
                let selection_anchor = tab.document.cursor().selection_anchor;
                let scroll_anchor = tab.scroll_anchor_state();
                let mut document = crate::document_view::DocumentView::from_external_content(
                    &path,
                    &content,
                    self.visible_rows(self.screen_height()),
                    self.visible_height_lines(self.screen_height()),
                );
                document.dirty = false;
                document.restore_edit_position(cursor_offset, selection_anchor, scroll_anchor);
                let (model, _) = document.into_parts();
                self.replace_editor_document(tab_id, model);
                if self.active_tab_id() == Some(tab_id)
                    && let Some(active_index) = self.active_editor_index()
                {
                    self.init_display_map(active_index);
                }
                self.refresh_file_monitor_roots();
                self.file_safety_notices
                    .push(crate::file_safety::FileSafetyNotice::CleanDocumentReloaded { path });
            }
            Ok(crate::file_safety::FileSafetyOutcome::Conflict { conflict, content, .. }) => {
                self.update_editor_document_path(tab_id, conflict.clone(), None);
                self.editor_runtime.forget_file_safety_tab(tab_id);
                self.file_safety_notices.push(
                    crate::file_safety::FileSafetyNotice::ConflictCopyCreated {
                        original: path.clone(),
                        conflict,
                    },
                );
                if self.active_tab_id() == Some(tab_id) {
                    let _ = self.open_external_content_tab(&path, &content);
                }
            }
            Ok(crate::file_safety::FileSafetyOutcome::Renamed { new_path, revision }) => {
                self.update_editor_document_path(tab_id, new_path.clone(), Some(revision));
                self.editor_runtime.forget_file_safety_tab(tab_id);
                self.refresh_file_monitor_roots();
            }
            Ok(crate::file_safety::FileSafetyOutcome::AmbiguousRename { original }) => {
                self.editor_runtime.forget_file_safety_tab(tab_id);
                self.detach_deleted_editor_document(tab_id, &original);
                self.refresh_file_monitor_roots();
                self.file_safety_notices
                    .push(crate::file_safety::FileSafetyNotice::AmbiguousRename { original });
            }
            Ok(crate::file_safety::FileSafetyOutcome::Deleted) => {
                self.editor_runtime.forget_file_safety_tab(tab_id);
                self.detach_deleted_editor_document(tab_id, &path);
                self.refresh_file_monitor_roots();
                self.file_safety_notices.push(
                    crate::file_safety::FileSafetyNotice::DocumentDetachedAfterDeletion {
                        original: path,
                    },
                );
            }
            Err(error) => {
                if expected_dirty {
                    self.file_safety_notices.push(
                        crate::file_safety::FileSafetyNotice::ConflictCopyFailed {
                            original: path,
                            message: error.to_string(),
                        },
                    );
                }
            }
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Safety: catch_unwind prevents panic from crossing the extern "C" boundary
        // in winit's macOS objc2 callbacks (Rust 2024 edition aborts on unwind).
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.do_resumed(event_loop);
        }));
        if let Err(e) = result {
            eprintln!("[fatal] panic in resumed: {:?}", e);
            event_loop.exit();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Accessibility(event) => {
                self.handle_accessibility_event(&event, event_loop);
            }
            event => {
                if self.handle_user_event(event) {
                    self.quit_app(event_loop);
                }
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.running = false;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // SAFETY: catch_unwind prevents panic from crossing the extern "C" boundary
        // in winit's macOS objc2 callbacks (Rust 2024 edition aborts on unwind).
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.do_window_event(event_loop, event);
        }));
        if let Err(e) = result {
            eprintln!("[fatal] panic in window_event: {:?}", e);
            event_loop.exit();
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let _atw_t0 = std::time::Instant::now();
        // Poll menu actions — menu clicks may happen between window events
        // Collect first to avoid borrow conflict with dispatch
        let actions: Vec<MenuAction> = if let Some(native_menu) = self.native_menu() {
            std::iter::from_fn(|| native_menu.poll_action()).collect()
        } else {
            Vec::new()
        };
        for action in actions {
            if action == MenuAction::Quit {
                self.quit_app(event_loop);
                return;
            }
            self.dispatch_menu_action(action, event_loop);
        }

        if self.editor_runtime.window().is_none() {
            return;
        }

        self.submit_file_safety_checks();

        #[cfg(debug_assertions)]
        let redraw_before = self.needs_redraw;
        #[cfg(debug_assertions)]
        let mut redraw_reason = "none";

        // 检测光标闪烁 phase 变化 → 触发渲染
        // 窗口未激活时不闪烁；搜索框焦点时跳过（TextBox 自己管理闪烁）
        // 预览模式没有可见光标，跳过闪烁检测
        if self.editor_runtime.window_focused()
            && !self.ui_shell.search_bar_has_keyboard_focus()
            && self.active_needs_cursor_blink_wakeup()
            && let Some(tab) = self.active_tab_session()
        {
            let (visible, _) = compute_cursor_phase(tab.cursor_render_state().cursor_blink_instant);
            if visible != self.last_cursor_visible {
                self.last_cursor_visible = visible;
                self.needs_redraw = true;
                #[cfg(debug_assertions)]
                {
                    redraw_reason = "blink";
                }
            }
        }

        // 检测动画 → 触发渲染
        if self.has_active_animation() {
            self.needs_redraw = true;
            #[cfg(debug_assertions)]
            {
                redraw_reason = if redraw_reason == "none" { "anim" } else { "blink+anim" };
            }
        }

        // 仅在有需要时 request_redraw（不再无条件刷新）
        #[cfg(debug_assertions)]
        let redraw_requested = self.needs_redraw;
        if self.needs_redraw
            && let Some(window) = self.editor_runtime.window()
        {
            window.request_redraw();
        }

        // 设置 ControlFlow — 精确调度下一次唤醒
        let next_wake = self.compute_next_wake_time();
        #[cfg(debug_assertions)]
        let wake_after_millis =
            next_wake.map(|d| d.duration_since(std::time::Instant::now()).as_millis() as i64);
        match next_wake {
            Some(deadline) => {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
            None => {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
        }

        let _atw_us = _atw_t0.elapsed().as_micros();
        // Debug-only: append about_to_wait timing to /tmp/perf.log
        #[cfg(debug_assertions)]
        {
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/perf.log")
                .and_then(|mut f| {
                    use std::io::Write;
                    writeln!(
                        f,
                        "[atw] {:>5}us nr={}->{} why={:<10} req={} wake={:?}",
                        _atw_us,
                        redraw_before,
                        self.needs_redraw,
                        redraw_reason,
                        redraw_requested,
                        wake_after_millis
                    )
                });
        }
        self.update_ime_cursor_area();
    }
}

#[cfg(test)]
mod file_safety_race_tests {
    use super::App;
    use crate::document_view::DocumentView;
    use crate::file_safety::{FileSafetyOutcome, capture_revision};
    use crate::plugins::editor::EditorPlugin;

    use std::fs;
    use std::time::Instant;

    #[test]
    fn stale_external_reload_is_discarded_and_rescheduled() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        fs::write(&path, "remote").expect("file should be written");
        let revision = capture_revision(&path).expect("revision should capture");

        let mut document = DocumentView::new(vec!["local edit".to_owned()], 10, 10.0);
        document.file_path = Some(path.clone());
        document.dirty = true;
        document.mark_content_changed();
        let mut app = App::new(None);
        app.push_entry_for_test(document, Box::new(EditorPlugin::new()));
        let tab_id = app.active_tab_id().expect("race fixture should have an active tab");

        app.apply_file_safety_outcome(
            tab_id,
            path.clone(),
            false,
            0,
            Ok(FileSafetyOutcome::Reload { content: "remote".to_owned(), revision }),
        );

        let entry = app.active_tab_session().expect("active entry should remain");
        assert_eq!(entry.document.full_text(), "local edit");
        assert_eq!(entry.document.file_path, Some(path.clone()));
        assert!(entry.document.dirty);
        assert!(app.editor_runtime.file_safety_should_check(Instant::now()));
    }

    #[test]
    fn stale_conflict_copy_is_not_rebound_to_the_tab() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        let conflict = directory.path().join("notes.textora-conflict.md");
        fs::write(&path, "remote").expect("file should be written");
        let revision = capture_revision(&path).expect("revision should capture");

        let mut document = DocumentView::new(vec!["latest local edit".to_owned()], 10, 10.0);
        document.file_path = Some(path.clone());
        document.dirty = true;
        document.mark_content_changed();
        let mut app = App::new(None);
        app.push_entry_for_test(document, Box::new(EditorPlugin::new()));
        let tab_id = app.active_tab_id().expect("race fixture should have an active tab");

        app.apply_file_safety_outcome(
            tab_id,
            path.clone(),
            true,
            0,
            Ok(FileSafetyOutcome::Conflict {
                conflict: conflict.clone(),
                content: "remote".to_owned(),
                revision,
            }),
        );

        let entry = app.active_tab_session().expect("active entry should remain");
        assert_eq!(entry.document.file_path, Some(path.clone()));
        assert_eq!(entry.document.full_text(), "latest local edit");
        assert!(app.file_safety_notices.iter().any(|notice| {
            matches!(
                notice,
                crate::file_safety::FileSafetyNotice::ConflictCopyCreated {
                    original,
                    conflict: notice_conflict,
                } if original == &path && notice_conflict == &conflict
            )
        }));
    }

    #[test]
    fn external_content_tab_is_product_prepared_and_returns_activation_effects() {
        let path = std::path::PathBuf::from("/virtual/shared.md");
        let mut app = App::new(None);
        app.new_untitled_doc();

        let effect = app.open_external_content_tab(&path, "# Shared");
        let active = app.active_tab_session().expect("external content should become active");

        assert!(effect.reshape);
        assert!(effect.redraw);
        assert!(effect.update_title);
        assert!(effect.persist_workspace);
        assert_eq!(active.document.file_path.as_ref(), Some(&path));
        assert_eq!(active.document.full_text(), "# Shared");
        assert_eq!(active.plugin_name(), ui::plugin::PLUGIN_MARKDOWN_EDITOR);
        assert_eq!(
            app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
            app.editor_runtime_tab_ids()
        );

        let external_tab_id = app.active_tab_id().expect("external content should have a tab ID");
        app.new_untitled_doc();
        let len_before_reopen = app.editor_tab_count();

        let reopen_effect = app.open_external_content_tab(&path, "# Replacement");
        let reopened =
            app.active_tab_session().expect("existing external content should reactivate");

        assert!(reopen_effect.reshape);
        assert!(reopen_effect.redraw);
        assert!(reopen_effect.update_title);
        assert!(reopen_effect.persist_workspace);
        assert_eq!(app.editor_tab_count(), len_before_reopen);
        assert_eq!(app.active_tab_id(), Some(external_tab_id));
        assert_eq!(reopened.document.full_text(), "# Shared");
        assert_eq!(
            app.editor_tab_ids_in_order().into_iter().collect::<std::collections::HashSet<_>>(),
            app.editor_runtime_tab_ids()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_direct_sync_controller_field_access(compact_source: &str) -> bool {
        let sync_controller_field = ["self.sync_", "controller"].concat();

        compact_source.match_indices(&sync_controller_field).any(|(index, _)| {
            let suffix = &compact_source[index + sync_controller_field.len()..];
            !suffix.starts_with("()") && !suffix.starts_with("_mut()")
        })
    }

    #[test]
    fn lifecycle_routes_sync_controller_access_through_app_accessors() {
        let production_source = include_str!("app_lifecycle.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("application lifecycle production source should precede tests");
        let compact_production_source = production_source.split_whitespace().collect::<String>();

        assert!(
            !has_direct_sync_controller_field_access(&compact_production_source),
            "application lifecycle must use App sync-controller accessors"
        );
    }

    #[test]
    fn sync_controller_boundary_rejects_fields_without_rejecting_accessors() {
        let sync_controller_field = ["self.sync_", "controller"].concat();

        assert!(has_direct_sync_controller_field_access(&format!(
            "{sync_controller_field}.as_ref()"
        )));
        assert!(has_direct_sync_controller_field_access(&format!("{sync_controller_field}=None;")));
        assert!(!has_direct_sync_controller_field_access(&format!("{sync_controller_field}()")));
        assert!(!has_direct_sync_controller_field_access(&format!(
            "{sync_controller_field}_mut()"
        )));
    }

    fn has_direct_native_menu_field_access(compact_source: &str) -> bool {
        let native_menu_field = ["self.native_", "menu"].concat();

        compact_source.match_indices(&native_menu_field).any(|(index, _)| {
            let suffix = &compact_source[index + native_menu_field.len()..];
            !suffix.starts_with("()")
        })
    }

    #[test]
    fn lifecycle_routes_native_menu_access_through_app_accessors() {
        let production_source = include_str!("app_lifecycle.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("application lifecycle production source should precede tests");
        let compact_production_source = production_source.split_whitespace().collect::<String>();

        assert!(
            !has_direct_native_menu_field_access(&compact_production_source),
            "application lifecycle must use App native-menu accessors"
        );
    }

    #[test]
    fn native_menu_boundary_rejects_fields_without_rejecting_accessors() {
        let native_menu_field = ["self.native_", "menu"].concat();

        assert!(has_direct_native_menu_field_access(&format!("{native_menu_field}.as_ref()")));
        assert!(has_direct_native_menu_field_access(&format!("{native_menu_field}=None;")));
        assert!(has_direct_native_menu_field_access(&native_menu_field));
        assert!(!has_direct_native_menu_field_access(&format!("{native_menu_field}()")));
        assert!(!has_direct_native_menu_field_access(
            "self.set_native_menu(NativeMenu::build_loading())"
        ));
    }

    #[test]
    fn user_event_routes_background_service_startup() {
        let production_source = include_str!("app_lifecycle.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("application lifecycle production source should precede tests");
        assert!(production_source.contains("AppEvent::StartBackgroundServices => {"));
        assert!(production_source.contains("self.start_background_services();"));
    }

    #[test]
    fn platform_accessibility_wraps_native_input_and_publishes_after_render() {
        let production_source = include_str!("app_lifecycle.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("application lifecycle production source should precede tests");
        let process_event = ["process_", "window_event"].concat();
        let render = ["self.", "render();"].concat();
        let publish_tree = ["self.publish_", "accessibility_tree();"].concat();

        let process_position =
            production_source.find(&process_event).expect("native event must reach AccessKit");
        let render_position = production_source.find(&render).expect("frame must render");
        let publish_position = production_source[render_position..]
            .find(&publish_tree)
            .map(|relative_position| render_position + relative_position)
            .expect("semantic tree must be published after rendering");

        assert!(process_position < render_position);
        assert!(render_position < publish_position);
    }

    #[test]
    fn product_save_paths_use_async_runtime_protocol() {
        let lifecycle_source = include_str!("app_lifecycle.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("application lifecycle production source should precede tests");
        let command_source = include_str!("dispatch/commands.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("command production source should precede tests");
        let tabs_source = include_str!("dispatch/tabs.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("tab command production source should precede tests");
        let synchronous_save_call = [".", "save_as("].concat();

        assert!(command_source.contains("submit_editor_save("));
        assert!(tabs_source.contains("submit_editor_save_before_close("));
        assert!(lifecycle_source.contains("drain_save_results("));
        assert!(!command_source.contains(&synchronous_save_call));
        assert!(!tabs_source.contains(&synchronous_save_call));
        assert!(!lifecycle_source.contains(&synchronous_save_call));
    }

    #[test]
    fn product_wake_requests_redraw_once_per_queued_sync_completion() {
        let mut app = App::new(None);
        app.needs_redraw = false;
        app.product
            .event_sender()
            .send_sync_results_ready()
            .expect("product inbox should accept sync completion");

        app.handle_user_event(AppEvent::ProductWake);

        assert!(app.needs_redraw);

        app.needs_redraw = false;
        app.handle_user_event(AppEvent::ProductWake);

        assert!(!app.needs_redraw);
    }
    use crate::actions::AppAction;
    use crate::app_dispatch::canvas_drag_test_support::{
        app_with_canvas_drag_tabs, cancel_request_count, document_texts, start_canvas_drag,
    };
    use crate::file_history::{FileHistory, FileHistoryEntry};
    use std::any::Any;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use ui::core::geom::Rect;
    use ui::core::widget::{Event, EventCtx, LayoutCtx, PaintCtx, Widget};
    use winit::keyboard::{Key, NamedKey};

    struct ModalInputWidget;

    impl Widget for ModalInputWidget {
        fn set_rect(&mut self, _rect: Rect, _ctx: &mut LayoutCtx) {}

        fn paint(&self, _ctx: &mut PaintCtx) {}

        fn hit(&self, _px: f32, _py: f32) -> bool {
            false
        }

        fn on_event(&mut self, event: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
            match event {
                Event::KeyDown(KeyCode::Char('k'), _) => Some(WidgetAction::Settings(
                    ui::settings_view::SettingsViewAction::SetFontFamily("keyboard".into()),
                )),
                Event::ImeCommit(_) => Some(WidgetAction::Settings(
                    ui::settings_view::SettingsViewAction::SetFontFamily("ime".into()),
                )),
                _ => None,
            }
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    struct WheelProbeWidget {
        rect: Rect,
        events: Rc<RefCell<Vec<Event>>>,
    }

    impl Widget for WheelProbeWidget {
        fn set_rect(&mut self, rect: Rect, _ctx: &mut LayoutCtx) {
            self.rect = rect;
        }

        fn paint(&self, _ctx: &mut PaintCtx) {}

        fn hit(&self, px: f32, py: f32) -> bool {
            self.rect.contains(px, py)
        }

        fn on_event(&mut self, event: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
            self.events.borrow_mut().push(event.clone());
            matches!(event, Event::Wheel { .. }).then_some(WidgetAction::Consumed)
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    struct ImeAllocationWidget {
        observed_allocation: Rc<Cell<Option<usize>>>,
    }

    impl Widget for ImeAllocationWidget {
        fn set_rect(&mut self, _rect: Rect, _ctx: &mut LayoutCtx) {}

        fn paint(&self, _ctx: &mut PaintCtx) {}

        fn hit(&self, _px: f32, _py: f32) -> bool {
            false
        }

        fn on_event(&mut self, event: &Event, _ctx: &mut EventCtx) -> Option<WidgetAction> {
            if let Event::ImeCommit(text) = event {
                self.observed_allocation.set(Some(text.as_ptr() as usize));
                return Some(WidgetAction::Consumed);
            }
            None
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }

    fn shell_with_modal_input_widget() -> crate::ui_shell::UiShell {
        let mut shell = crate::ui_shell::UiShell::new();
        shell.push_overlay_with_policy(
            Box::new(ui::modal_frame::ModalFrame::new("Settings", Box::new(ModalInputWidget))),
            ui::OverlayLayout::Fixed(Rect::new(0.0, 0.0, 400.0, 300.0)),
            ui::OverlayInputPolicy::Modal,
            ui::DismissPolicy::EscapeOrExplicit,
        );
        shell
    }

    fn shell_with_wheel_probe(events: Rc<RefCell<Vec<Event>>>) -> crate::ui_shell::UiShell {
        let mut shell = crate::ui_shell::UiShell::new();
        shell.push_overlay_with_policy(
            Box::new(ui::modal_frame::ModalFrame::new(
                "Settings",
                Box::new(WheelProbeWidget { rect: Rect::ZERO, events }),
            )),
            ui::OverlayLayout::Fixed(Rect::new(0.0, 0.0, 400.0, 300.0)),
            ui::OverlayInputPolicy::Modal,
            ui::DismissPolicy::EscapeOrExplicit,
        );
        shell
    }

    fn layout_modal_overlay(shell: &mut crate::ui_shell::UiShell, theme: &ui::Theme) {
        let mut measure = ui::core::measure::NoopMeasure;
        let mut ctx = LayoutCtx { ui_measure: None, measure: &mut measure, theme, dpi: 1.0 };
        let frame = shell
            .active_overlay_widget_mut::<ui::modal_frame::ModalFrame>()
            .expect("test shell should contain a modal frame");
        frame.set_rect(Rect::new(0.0, 0.0, 400.0, 300.0), &mut ctx);
    }

    #[test]
    fn modal_ime_route_reuses_the_original_text_allocation() {
        let theme = ui::theme::test_theme();
        let observed_allocation = Rc::new(Cell::new(None));
        let mut shell = crate::ui_shell::UiShell::new();
        shell.push_overlay_with_policy(
            Box::new(ui::modal_frame::ModalFrame::new(
                "Settings",
                Box::new(ImeAllocationWidget {
                    observed_allocation: Rc::clone(&observed_allocation),
                }),
            )),
            ui::OverlayLayout::Fixed(Rect::new(0.0, 0.0, 400.0, 300.0)),
            ui::OverlayInputPolicy::Modal,
            ui::DismissPolicy::EscapeOrExplicit,
        );
        let text = "modal-sensitive-ime-route".to_owned();
        let original_allocation = text.as_ptr() as usize;

        let route = route_modal_ime_input(&mut shell, Event::ImeCommit(text), &theme, 1.0);

        assert!(matches!(route, Some(ModalInputRoute::Redraw)));
        assert_eq!(observed_allocation.get(), Some(original_allocation));
    }

    #[test]
    fn active_modal_routes_keyboard_ime_and_dismiss_to_application_actions() {
        let theme = ui::theme::test_theme();
        let mut shell = shell_with_modal_input_widget();

        let keyboard_route = route_modal_keyboard_input(
            &mut shell,
            Some(KeyCode::Char('k')),
            Modifiers::NONE,
            &theme,
            1.0,
        )
        .expect("active modal keyboard input must be routed");
        assert!(matches!(
            keyboard_route,
            ModalInputRoute::Dispatch(AppAction::Settings(
                ui::settings_view::SettingsViewAction::SetFontFamily(font_family),
            )) if font_family == "keyboard"
        ));

        let ime_route =
            route_modal_ime_input(&mut shell, Event::ImeCommit("拼音".into()), &theme, 1.0)
                .expect("active modal IME input must be routed");
        assert!(matches!(
            ime_route,
            ModalInputRoute::Dispatch(AppAction::Settings(
                ui::settings_view::SettingsViewAction::SetFontFamily(font_family),
            )) if font_family == "ime"
        ));

        let dismiss_route = route_modal_keyboard_input(
            &mut shell,
            Some(KeyCode::Escape),
            Modifiers::NONE,
            &theme,
            1.0,
        )
        .expect("active modal escape input must be routed");
        assert!(matches!(dismiss_route, ModalInputRoute::Dispatch(AppAction::DismissOverlay)));
    }

    #[test]
    fn active_modal_routes_native_wheel_to_modal_overlay() {
        let theme = ui::theme::test_theme();
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut shell = shell_with_wheel_probe(Rc::clone(&events));
        layout_modal_overlay(&mut shell, &theme);
        let content_rect = shell
            .active_overlay_widget_ref::<ui::modal_frame::ModalFrame>()
            .expect("test shell should contain a modal frame")
            .content_rect();

        let route = route_modal_wheel_delta(
            &mut shell,
            &winit::event::MouseScrollDelta::LineDelta(1.0, -2.0),
            (200.0, 160.0),
            20.0,
            &theme,
            1.0,
        );

        assert!(matches!(route, Some(ModalInputRoute::Redraw)));
        assert_eq!(
            events.borrow().as_slice(),
            [Event::Wheel {
                dx: 60.0,
                dy: -120.0,
                px: 200.0 - content_rect.x,
                py: 160.0 - content_rect.y,
            }]
        );

        let route = route_modal_wheel_delta(
            &mut shell,
            &winit::event::MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(
                4.0, -8.0,
            )),
            (200.0, 160.0),
            20.0,
            &theme,
            1.0,
        );
        assert!(matches!(route, Some(ModalInputRoute::Redraw)));
        assert_eq!(
            events.borrow()[1],
            Event::Wheel {
                dx: 4.0,
                dy: -8.0,
                px: 200.0 - content_rect.x,
                py: 160.0 - content_rect.y,
            }
        );
    }

    #[test]
    fn no_modal_does_not_claim_wheel_input() {
        let theme = ui::theme::test_theme();
        let mut shell = crate::ui_shell::UiShell::new();

        assert!(
            route_modal_wheel_delta(
                &mut shell,
                &winit::event::MouseScrollDelta::LineDelta(0.0, -1.0),
                (100.0, 100.0),
                20.0,
                &theme,
                1.0,
            )
            .is_none()
        );
    }

    #[test]
    fn modal_wheel_event_converts_native_delta_to_ui_pixels() {
        let line_event = modal_wheel_event(
            &winit::event::MouseScrollDelta::LineDelta(1.0, -2.0),
            (120.0, 80.0),
            20.0,
        );
        assert_eq!(line_event, Event::Wheel { dx: 60.0, dy: -120.0, px: 120.0, py: 80.0 },);

        let pixel_event = modal_wheel_event(
            &winit::event::MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(
                4.0, -8.0,
            )),
            (120.0, 80.0),
            20.0,
        );
        assert_eq!(pixel_event, Event::Wheel { dx: 4.0, dy: -8.0, px: 120.0, py: 80.0 },);
    }

    #[test]
    fn active_modal_consumes_unconvertible_keyboard_input() {
        let theme = ui::theme::test_theme();
        let mut shell = shell_with_modal_input_widget();

        let route = route_modal_keyboard_input(&mut shell, None, Modifiers::NONE, &theme, 1.0)
            .expect("active modal must consume every keyboard input");

        assert!(matches!(route, ModalInputRoute::Redraw));
    }

    #[test]
    fn no_modal_does_not_claim_unconvertible_keyboard_input() {
        let theme = ui::theme::test_theme();
        let mut shell = crate::ui_shell::UiShell::new();

        assert!(
            route_modal_keyboard_input(&mut shell, None, Modifiers::NONE, &theme, 1.0).is_none()
        );
    }

    fn shell_with_mindmap_style_panel_focus() -> crate::ui_shell::UiShell {
        let theme = ui::theme::test_theme();
        let mut measure = ui::core::NoopMeasure;
        let mut app = App::new(None);
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
        app.ui_shell
    }

    #[test]
    fn mindmap_style_panel_keyboard_route_dispatches_panel_actions_before_editor() {
        let theme = ui::theme::test_theme();
        let mut shell = shell_with_mindmap_style_panel_focus();

        let move_route = route_mindmap_style_panel_keyboard_input(
            &mut shell,
            Some(KeyCode::Right),
            Modifiers::NONE,
            &theme,
            1.0,
        );
        assert!(matches!(move_route, Some(FocusedWidgetInputRoute::Redraw)));

        let select_route = route_mindmap_style_panel_keyboard_input(
            &mut shell,
            Some(KeyCode::Enter),
            Modifiers::NONE,
            &theme,
            1.0,
        );
        assert!(matches!(
            select_route,
            Some(FocusedWidgetInputRoute::Dispatch(AppAction::MindmapStylePanel(
                ui::core::widget::MindmapStylePanelAction::SelectTheme(theme_id)
            ))) if theme_id == "dawn"
        ));

        let close_route = route_mindmap_style_panel_keyboard_input(
            &mut shell,
            Some(KeyCode::Escape),
            Modifiers::NONE,
            &theme,
            1.0,
        );
        assert!(matches!(
            close_route,
            Some(FocusedWidgetInputRoute::Dispatch(AppAction::MindmapStylePanel(
                ui::core::widget::MindmapStylePanelAction::Close
            )))
        ));
    }

    #[test]
    fn mindmap_style_panel_keyboard_route_consumes_unhandled_keys_but_not_editor_focus() {
        let theme = ui::theme::test_theme();
        let mut shell = shell_with_mindmap_style_panel_focus();

        assert!(matches!(
            route_mindmap_style_panel_keyboard_input(
                &mut shell,
                Some(KeyCode::Char('x')),
                Modifiers::NONE,
                &theme,
                1.0,
            ),
            Some(FocusedWidgetInputRoute::Consumed)
        ));

        shell.focus_editor();
        assert!(
            route_mindmap_style_panel_keyboard_input(
                &mut shell,
                Some(KeyCode::Enter),
                Modifiers::NONE,
                &theme,
                1.0,
            )
            .is_none()
        );
    }

    #[test]
    fn mindmap_style_panel_keyboard_route_bypasses_global_shortcuts() {
        let mut shell = shell_with_mindmap_style_panel_focus();

        assert!(mindmap_style_panel_should_receive_keyboard(
            &shell,
            &Key::Character("x".into()),
            false,
            false,
            false,
            false,
        ));
        for logical_key in [
            Key::Character("s".into()),
            Key::Character("w".into()),
            Key::Character("t".into()),
            Key::Character("o".into()),
            Key::Character("1".into()),
            Key::Character("b".into()),
        ] {
            assert!(!mindmap_style_panel_should_receive_keyboard(
                &shell,
                &logical_key,
                true,
                false,
                false,
                false,
            ));
        }
        assert!(!mindmap_style_panel_should_receive_keyboard(
            &shell,
            &Key::Character("p".into()),
            true,
            true,
            false,
            false,
        ));
        assert!(mindmap_style_panel_should_receive_keyboard(
            &shell,
            &Key::Character("q".into()),
            true,
            false,
            false,
            false,
        ));
        assert!(!mindmap_style_panel_should_receive_keyboard(
            &shell,
            &Key::Named(NamedKey::ArrowLeft),
            false,
            false,
            true,
            false,
        ));
        assert!(!mindmap_style_panel_should_receive_keyboard(
            &shell,
            &Key::Character("z".into()),
            false,
            false,
            false,
            true,
        ));

        shell.focus_editor();
        assert!(!mindmap_style_panel_should_receive_keyboard(
            &shell,
            &Key::Character("x".into()),
            false,
            false,
            false,
            false,
        ));
    }

    #[test]
    fn mindmap_style_panel_ime_route_consumes_preedit_and_commit_only() {
        let mut shell = shell_with_mindmap_style_panel_focus();
        let preedit = Event::ImePreedit { text: "拼".into(), cursor: Some((0, 3)) };
        let commit = Event::ImeCommit("拼".into());

        assert!(mindmap_style_panel_consumes_ime(&shell, &preedit));
        assert!(mindmap_style_panel_consumes_ime(&shell, &commit));
        assert!(!mindmap_style_panel_consumes_ime(&shell, &Event::ImeEnable));
        assert!(!mindmap_style_panel_consumes_ime(&shell, &Event::ImeDisable));

        shell.focus_editor();
        assert!(!mindmap_style_panel_consumes_ime(&shell, &preedit));
        assert!(!mindmap_style_panel_consumes_ime(&shell, &commit));
    }

    #[test]
    fn settings_overlay_text_box_receives_clicks_and_typed_input_end_to_end() {
        use ui::core::Screen;
        use ui::core::paint::DrawCmd;
        use winit::event::ElementState;

        let theme = ui::theme::test_theme();
        let mut app = crate::app::App::new(None);
        app.open_settings_overlay();

        let mut measure = ui::core::measure::NoopMeasure;
        let inputs = app.build_shell_inputs();
        app.ui_shell.mark_layout_initialized_for_test();
        app.ui_shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut measure, &inputs);

        let mut shaper = shaping::Shaper::new().expect("test shaper should initialize");
        let draw = app.ui_shell.paint_chrome(&theme, 1.0, Some(&mut shaper));
        let text_index = draw
            .cmds
            .iter()
            .rposition(|command| {
                matches!(command, DrawCmd::TextLayout { layout, .. } if layout.text == "15")
            })
            .expect("settings overlay should paint the font size text box");
        let text_box_rect = draw.cmds[..text_index]
            .iter()
            .rev()
            .find_map(|command| match command {
                DrawCmd::FillRect { rect, radius, .. } if *radius == 3.0 => Some(*rect),
                _ => None,
            })
            .expect("expected text box background before its text");
        let click_x = text_box_rect.x + text_box_rect.w * 0.5;
        let click_y = text_box_rect.y + text_box_rect.h * 0.5;

        let press_actions = crate::events::handle_mouse_input_left(
            &mut app,
            ElementState::Pressed,
            click_x,
            click_y,
        );
        let release_actions = crate::events::handle_mouse_input_left(
            &mut app,
            ElementState::Released,
            click_x,
            click_y,
        );
        assert!(press_actions.is_empty(), "点击输入框不应产生应用级动作");
        assert!(release_actions.is_empty(), "松开输入框不应产生应用级动作");

        let select_all_route = route_modal_keyboard_input(
            &mut app.ui_shell,
            Some(KeyCode::Char('a')),
            Modifiers { cmd: true, ..Modifiers::NONE },
            &theme,
            1.0,
        );
        assert!(matches!(select_all_route, Some(ModalInputRoute::Redraw)));

        let typed_route = route_modal_keyboard_input(
            &mut app.ui_shell,
            Some(KeyCode::Char('9')),
            Modifiers::NONE,
            &theme,
            1.0,
        );
        assert!(
            matches!(typed_route, Some(ModalInputRoute::Redraw)),
            "聚焦后输入字符应被输入框消费并重绘",
        );

        let commit_route = route_modal_keyboard_input(
            &mut app.ui_shell,
            Some(KeyCode::Enter),
            Modifiers::NONE,
            &theme,
            1.0,
        );
        assert!(
            matches!(
                commit_route,
                Some(ModalInputRoute::Dispatch(AppAction::Settings(
                    ui::settings_view::SettingsViewAction::SetFontSize(size),
                ))) if size == 9.0
            ),
            "回车应提交输入框中的新字号",
        );
    }

    #[test]
    fn modal_settings_action_is_dispatched_instead_of_only_redrawing() {
        let action = WidgetAction::Settings(ui::settings_view::SettingsViewAction::SetFontFamily(
            "Menlo".into(),
        ));

        assert!(matches!(
            modal_widget_action(&action),
            Some(AppAction::Settings(
                ui::settings_view::SettingsViewAction::SetFontFamily(font_family),
            )) if font_family == "Menlo"
        ));
    }

    #[test]
    fn modal_dismiss_action_is_dispatched_instead_of_only_redrawing() {
        let action = WidgetAction::Overlay(ui::core::overlay::OverlayAction::DismissRequested);

        assert!(matches!(modal_widget_action(&action), Some(AppAction::DismissOverlay)));
    }

    // ── winit_key_to_keycode ──

    #[test]
    fn keycode_char_from_text() {
        assert_eq!(
            winit_key_to_keycode(&Key::Character("a".into()), Some("a")),
            Some(KeyCode::Char('a'))
        );
    }

    #[test]
    fn keycode_char_shift_handled_by_text() {
        // event.text handles Shift+1 → "!" naturally; our function just passes text through
        assert_eq!(
            winit_key_to_keycode(&Key::Character("1".into()), Some("!")),
            Some(KeyCode::Char('!'))
        );
    }

    #[test]
    fn keycode_non_english_layout() {
        // On German keyboard, Shift+8 → "("
        assert_eq!(
            winit_key_to_keycode(&Key::Character("8".into()), Some("(")),
            Some(KeyCode::Char('('))
        );
    }

    #[test]
    fn keycode_named_tab_prefers_semantic_key_over_event_text() {
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::Tab), Some("\t")),
            Some(KeyCode::Tab)
        );
    }

    #[test]
    fn recent_file_loader_filters_missing_paths() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let workspace = directory.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace directory should be created");
        let existing_path = workspace.join("existing.md");
        std::fs::write(&existing_path, "# existing\n")
            .expect("existing recent file should be written");
        let missing_path = workspace.join("missing.md");
        let mut history = FileHistory::default();
        history.record(FileHistoryEntry {
            file_path: existing_path.clone(),
            workspace_root: Some(workspace.clone()),
            last_closed_at: 2_000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        history.record(FileHistoryEntry {
            file_path: missing_path,
            workspace_root: Some(workspace.clone()),
            last_closed_at: 1_000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });

        let recent_paths = load_recent_file_paths(&history, Some(&workspace));

        assert_eq!(recent_paths, vec![existing_path]);
    }

    #[test]
    fn recent_file_loader_queues_paths_and_emits_product_wake() {
        use std::sync::mpsc;
        use std::time::Duration;

        use appkit_shell::ProductHost;

        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let workspace = directory.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace directory should be created");
        let existing_path = workspace.join("existing.md");
        std::fs::write(&existing_path, "# existing\n")
            .expect("existing recent file should be written");
        let mut history = FileHistory::default();
        history.record(FileHistoryEntry {
            file_path: existing_path,
            workspace_root: Some(workspace.clone()),
            last_closed_at: 2_000,
            last_cursor_line: 0,
            last_cursor_col: 0,
            scroll_anchor_line: 0,
            scroll_anchor_offset: 0.0,
        });
        let mut product = crate::textora_product::TextoraProduct::new();
        let (wake_sender, wake_receiver) = mpsc::channel();

        spawn_recent_file_loader(
            move |event| wake_sender.send(event).is_ok(),
            product.event_sender(),
            history,
            Some(workspace),
        );

        let event = wake_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("recent file loader should wake the app");
        assert!(matches!(event, AppEvent::ProductWake));

        ProductHost::drain_product_events(&mut product);
        assert!(product.native_menu().is_some());
    }

    #[test]
    fn keycode_named_escape() {
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::Escape), None),
            Some(KeyCode::Escape)
        );
    }

    #[test]
    fn keycode_named_enter() {
        assert_eq!(winit_key_to_keycode(&Key::Named(NamedKey::Enter), None), Some(KeyCode::Enter));
    }

    #[test]
    fn keycode_named_backspace() {
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::Backspace), None),
            Some(KeyCode::Backspace)
        );
    }

    #[test]
    fn keycode_named_delete() {
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::Delete), None),
            Some(KeyCode::Delete)
        );
    }

    #[test]
    fn keycode_named_arrows() {
        assert_eq!(winit_key_to_keycode(&Key::Named(NamedKey::ArrowUp), None), Some(KeyCode::Up));
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::ArrowDown), None),
            Some(KeyCode::Down)
        );
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::ArrowLeft), None),
            Some(KeyCode::Left)
        );
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::ArrowRight), None),
            Some(KeyCode::Right)
        );
    }

    #[test]
    fn keycode_named_home_end() {
        assert_eq!(winit_key_to_keycode(&Key::Named(NamedKey::Home), None), Some(KeyCode::Home));
        assert_eq!(winit_key_to_keycode(&Key::Named(NamedKey::End), None), Some(KeyCode::End));
    }

    #[test]
    fn keycode_named_page_up_down() {
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::PageUp), None),
            Some(KeyCode::PageUp)
        );
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::PageDown), None),
            Some(KeyCode::PageDown)
        );
    }

    #[test]
    fn keycode_unknown_named_returns_none() {
        assert_eq!(winit_key_to_keycode(&Key::Named(NamedKey::F1), None), None);
    }

    #[test]
    fn keycode_empty_text_falls_through_to_named() {
        // F3 has no text, but is a Named key that we don't map
        assert_eq!(winit_key_to_keycode(&Key::Named(NamedKey::F3), None), None);
    }

    #[test]
    fn keycode_text_takes_priority_over_named() {
        // Text path runs first; even if logical_key is Enter, text takes priority
        assert_eq!(
            winit_key_to_keycode(&Key::Named(NamedKey::Enter), Some("x")),
            Some(KeyCode::Char('x'))
        );
    }

    // ── is_search_bar_whitelist ──

    #[test]
    fn whitelist_cmd_f() {
        assert!(is_search_bar_whitelist(&Key::Character("f".into()), true, false, false));
    }

    #[test]
    fn whitelist_cmd_shift_f() {
        assert!(is_search_bar_whitelist(&Key::Character("f".into()), true, true, false));
    }

    #[test]
    fn whitelist_cmd_s() {
        assert!(is_search_bar_whitelist(&Key::Character("s".into()), true, false, false));
    }

    #[test]
    fn whitelist_cmd_shift_s() {
        assert!(is_search_bar_whitelist(&Key::Character("s".into()), true, true, false));
    }

    #[test]
    fn whitelist_cmd_w() {
        assert!(is_search_bar_whitelist(&Key::Character("w".into()), true, false, false));
    }

    #[test]
    fn whitelist_cmd_z() {
        assert!(is_search_bar_whitelist(&Key::Character("z".into()), true, false, false));
    }

    #[test]
    fn whitelist_cmd_shift_z() {
        assert!(is_search_bar_whitelist(&Key::Character("z".into()), true, true, false));
    }

    #[test]
    fn whitelist_cmd_bracket() {
        assert!(is_search_bar_whitelist(&Key::Character("[".into()), true, false, false));
        assert!(is_search_bar_whitelist(&Key::Character("]".into()), true, false, false));
    }

    #[test]
    fn whitelist_cmd_shift_bracket() {
        assert!(is_search_bar_whitelist(&Key::Character("[".into()), true, true, false));
        assert!(is_search_bar_whitelist(&Key::Character("]".into()), true, true, false));
    }

    #[test]
    fn whitelist_cmd_alt_arrows() {
        assert!(is_search_bar_whitelist(&Key::Named(NamedKey::ArrowLeft), true, false, true));
        assert!(is_search_bar_whitelist(&Key::Named(NamedKey::ArrowRight), true, false, true));
    }

    #[test]
    fn whitelist_no_super_returns_false() {
        assert!(!is_search_bar_whitelist(&Key::Character("f".into()), false, false, false));
        assert!(!is_search_bar_whitelist(&Key::Character("f".into()), false, true, false));
    }

    #[test]
    fn whitelist_unknown_char_not_whitelisted() {
        assert!(!is_search_bar_whitelist(&Key::Character("g".into()), true, false, false));
    }

    #[test]
    fn whitelist_arrow_without_alt_not_whitelisted() {
        assert!(!is_search_bar_whitelist(&Key::Named(NamedKey::ArrowLeft), true, false, false));
    }

    #[test]
    fn whitelist_cmd_shift_arrow_not_whitelisted() {
        // Cmd+Shift+Alt+Left is not whitelisted (whitelist requires !shift for arrows)
        assert!(!is_search_bar_whitelist(&Key::Named(NamedKey::ArrowLeft), true, true, true));
    }

    #[test]
    fn scale_factor_round_trip_preserves_logical_persistence_value() {
        let mut app = App::new(None);
        let logical = app.persisted_font_size();

        app.handle_scale_factor_changed(2.0);
        assert_eq!(app.persisted_font_size(), logical);
        app.handle_scale_factor_changed(1.0);
        assert_eq!(app.persisted_font_size(), logical);
    }

    #[test]
    fn keycode_cmd_char_falls_back_to_logical_character_when_text_is_control() {
        assert_eq!(
            winit_key_to_keycode(&Key::Character("a".into()), Some("\x01")),
            Some(KeyCode::Char('a'))
        );
    }

    #[test]
    fn keycode_cmd_char_falls_back_to_logical_character_when_text_is_none() {
        assert_eq!(
            winit_key_to_keycode(&Key::Character("c".into()), None),
            Some(KeyCode::Char('c'))
        );
    }

    #[test]
    fn search_focus_does_not_whitelist_cmd_a_c_x_v() {
        assert!(!is_search_bar_whitelist(&Key::Character("a".into()), true, false, false));
        assert!(!is_search_bar_whitelist(&Key::Character("c".into()), true, false, false));
        assert!(!is_search_bar_whitelist(&Key::Character("x".into()), true, false, false));
        assert!(!is_search_bar_whitelist(&Key::Character("v".into()), true, false, false));
    }

    #[test]
    fn search_focus_routes_delete_to_widget() {
        // App creation requires an EventLoop in some environments, but App::new(None) works in tests
        let mut app = App::new(None);
        app.ui_shell.focus_widget(ui::core::widget::ids::SEARCH_BAR);

        // This is a unit test that verifies `is_search_bar_whitelist` for Delete returns false
        assert!(!is_search_bar_whitelist(&Key::Named(NamedKey::Delete), false, false, false));
    }

    #[test]
    fn dropped_file_applies_open_effect_immediately() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("dropped.md");
        std::fs::write(&path, "# dropped file\n")
            .expect("temporary markdown file should be written");
        let mut app = App::new(None);
        let generation_before = app.editor_runtime.reshape_generation();
        app.needs_redraw = false;

        app.handle_dropped_file(&path);

        assert_eq!(app.editor_runtime.reshape_generation(), generation_before + 1);
        assert!(app.needs_redraw);
    }

    #[test]
    fn macos_open_file_requests_continue_after_an_invalid_path() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let valid_path = directory.path().join("valid.md");
        std::fs::write(&valid_path, "# valid\n")
            .expect("temporary markdown file should be written");
        let mut app = App::new(None);
        app.replace_editor_model(
            crate::app_init::build_product_workspace(),
            crate::tab_runtime::TabRuntimeStore::default(),
        );

        app.open_document_sender()
            .send(vec![directory.path().join("missing.md"), valid_path.clone()])
            .expect("app owns the product open-document receiver");
        app.handle_user_event(AppEvent::ProductWake);

        assert_eq!(app.editor_tab_count(), 1);
        assert_eq!(
            app.active_tab_session().and_then(|session| session.document.file_path.as_deref()),
            Some(valid_path.as_path())
        );
    }

    #[test]
    fn window_focus_loss_cancels_started_canvas_drag_once() {
        let (mut app, state) = app_with_canvas_drag_tabs();
        start_canvas_drag(&mut app);

        let effect = app.handle_window_focus_changed(false);

        assert!(!app.editor_runtime.window_focused());
        assert!(effect.redraw);
        assert_eq!(cancel_request_count(&state.borrow()), 1);
        assert_eq!(document_texts(&app), ["abc", "def"]);

        app.handle_window_focus_changed(false);

        assert_eq!(cancel_request_count(&state.borrow()), 1);
    }

    #[test]
    fn window_focus_loss_clears_text_selection_capture_preedit_and_mouse_state() {
        let mut app = App::new(None);
        app.replace_editor_model(
            crate::app_init::build_product_workspace(),
            crate::tab_runtime::TabRuntimeStore::default(),
        );
        let context = editor_input_context(&app);
        assert!(app.editor_runtime.begin_text_selection(context));
        assert!(app.editor_runtime.update_preedit(context, "未完成".to_owned(), Some((0, 6)),));
        app.mouse.is_down = true;
        app.mouse.down_byte_offset = Some(0);
        app.mouse.wysiwyg_selection_scope = Some(0..1);
        app.mouse.last_hover_redraw_pos = Some((10.0, 20.0));
        app.mouse.last_hover_tab = Some(0);

        let effect = app.handle_window_focus_changed(false);

        assert!(effect.redraw);
        assert!(!app.editor_runtime.window_focused());
        assert_eq!(
            app.editor_runtime.pointer_capture(),
            appkit_shell::editor_runtime::MouseCapture::None
        );
        assert_eq!(app.editor_runtime.preedit(), (String::new(), None));
        assert!(!app.mouse.is_down);
        assert_eq!(app.mouse.down_byte_offset, None);
        assert_eq!(app.mouse.wysiwyg_selection_scope, None);
        assert_eq!(app.mouse.last_hover_redraw_pos, None);
        assert_eq!(app.mouse.last_hover_tab, None);

        app.handle_window_focus_changed(false);
        assert_eq!(
            app.editor_runtime.pointer_capture(),
            appkit_shell::editor_runtime::MouseCapture::None
        );
        assert_eq!(app.editor_runtime.preedit(), (String::new(), None));
    }

    #[test]
    fn canvas_pinch_action_uses_mouse_position_and_ignores_nan() {
        let action = canvas_pinch_action(0.25, (320.0, 240.0));
        assert!(matches!(
            action,
            Some(AppAction::CanvasPinch { delta, screen_anchor })
                if delta == 0.25 && screen_anchor == ui::canvas::CanvasPoint::new(320.0, 240.0)
        ));
        assert!(canvas_pinch_action(f64::NAN, (320.0, 240.0)).is_none());
    }
}
