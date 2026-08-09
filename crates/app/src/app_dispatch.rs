//! Command dispatch: menu actions, keyboard commands, sidebar actions.
//! Methods on `impl App`, extracted from app.rs.

use crate::actions::AppAction;
use crate::app::App;
use crate::app_effect::AppEffect;
use crate::canvas_viewport::CanvasViewportAction;
use crate::dispatch::chrome::{ChromeDispatchAction, SettingsDispatchAction, TabScrollDirection};
use crate::dispatch::viewport::ViewportDispatchAction;
use crate::sync_settings_types::SyncSettingsAction;
use appkit_core::workspace::types::TabId;
use appkit_shell::DrainStart;
use textora_sync::{DeviceId, LoopbackEndpoint, RemoteDeviceSpec, StaticSyncAddress};

const PREVIEW_TOP_PAD_LOGICAL: f32 = 16.0;
const PREVIEW_HORIZONTAL_PAD_LOGICAL: f32 = 20.0;
const MAX_READING_WIDTH_LOGICAL: f32 = 800.0;
const MIN_PLUGIN_VIEWPORT_PX: f32 = 100.0;
const MIN_CANVAS_PINCH_ZOOM_FACTOR: f32 = 0.01;
const SYNC_INVALID_ENDPOINT_NOTICE: &str = "Syncthing 地址无效";
const SYNC_INVALID_API_KEY_NOTICE: &str = "API Key 无效";
const SYNC_INVALID_REMOTE_DEVICE_NOTICE: &str = "远端设备信息无效";
const SYNC_INVALID_REMOTE_ADDRESS_NOTICE: &str = "远端设备地址无效";
const SYNC_REMOTE_NAME_REQUIRED_NOTICE: &str = "远端设备名称不能为空";
const SYNC_LIBRARY_NOT_FOUND_NOTICE: &str = "资料库状态已更新，请重试";
const SYNC_PENDING_FOLDER_NOT_FOUND_NOTICE: &str = "待接收资料库不存在";

fn parse_sync_endpoint(candidate: &str) -> Result<LoopbackEndpoint, &'static str> {
    LoopbackEndpoint::parse(candidate.trim()).map_err(|_| SYNC_INVALID_ENDPOINT_NOTICE)
}

fn parse_sync_remote_device(
    device_id: String,
    name: String,
    raw_addresses: String,
) -> Result<RemoteDeviceSpec, &'static str> {
    let device_id = DeviceId::parse(device_id.trim().to_owned())
        .map_err(|_| SYNC_INVALID_REMOTE_DEVICE_NOTICE)?;
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(SYNC_REMOTE_NAME_REQUIRED_NOTICE);
    }

    let mut addresses = Vec::new();
    for address in raw_addresses.split(',').map(str::trim).filter(|value| !value.is_empty()) {
        if address == textora_sync::SYNCTHING_DYNAMIC_ADDRESS {
            continue;
        }
        addresses.push(
            StaticSyncAddress::parse(address.to_owned())
                .map_err(|_| SYNC_INVALID_REMOTE_ADDRESS_NOTICE)?,
        );
    }

    Ok(RemoteDeviceSpec { device_id, name, addresses })
}

fn sync_controller_error_notice(
    error: &crate::sync_controller::SyncControllerError,
) -> &'static str {
    match error {
        crate::sync_controller::SyncControllerError::InvalidApiKey => SYNC_INVALID_API_KEY_NOTICE,
        crate::sync_controller::SyncControllerError::WorkerUnavailable => "同步服务暂不可用",
    }
}

fn take_top(rect: &mut ui::core::geom::Rect, thickness: f32) {
    let clamped = thickness.max(0.0).min(rect.h);
    rect.y += clamped;
    rect.h = (rect.h - clamped).max(0.0);
}

fn take_bottom(rect: &mut ui::core::geom::Rect, thickness: f32) {
    let clamped = thickness.max(0.0).min(rect.h);
    rect.h = (rect.h - clamped).max(0.0);
}

fn take_left(rect: &mut ui::core::geom::Rect, thickness: f32) {
    let clamped = thickness.max(0.0).min(rect.w);
    rect.x += clamped;
    rect.w = (rect.w - clamped).max(0.0);
}

fn take_right(rect: &mut ui::core::geom::Rect, thickness: f32) {
    let clamped = thickness.max(0.0).min(rect.w);
    rect.w = (rect.w - clamped).max(0.0);
}

fn projected_editor_rect(
    screen_w: f32,
    screen_h: f32,
    inputs: &crate::ui_shell::ShellInputs,
    mindmap_style_panel_thickness: f32,
) -> ui::core::geom::Rect {
    let mut rect = ui::core::geom::Rect::new(0.0, 0.0, screen_w, screen_h);

    if inputs.tabs_visible {
        take_top(&mut rect, inputs.tabs_thickness);
    }
    if inputs.sidebar_visible && inputs.sidebar_thickness > 0.0 {
        take_left(&mut rect, inputs.sidebar_thickness);
    }
    if inputs.sidebar_visible {
        take_top(&mut rect, ui::title_bar::title_bar_height(inputs.metrics.dpi));
    }
    if inputs.toc_visible {
        take_left(&mut rect, inputs.toc_thickness);
    }
    if inputs.search_visible {
        take_top(&mut rect, inputs.search_thickness);
    }
    take_bottom(&mut rect, inputs.status_thickness);
    take_right(&mut rect, mindmap_style_panel_thickness);
    take_right(&mut rect, inputs.scrollbar_thickness);

    rect
}

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
        let commands = crate::menu_handler::dispatch_menu_action(action);
        self.dispatch(AppAction::ExecuteAppCommands(commands), event_loop);
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
    fn reduce_action(
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
            AppAction::Sync(action) => self.dispatch_sync_settings_action(action),
            AppAction::UpdateMousePos(x, y) => {
                self.mouse.pos = (x, y);
                AppEffect::NONE
            }
            AppAction::HandleScroll(delta) => self.dispatch_wheel_scroll(delta),
            AppAction::EditorMouseInput { state, px, py, hit } => {
                self.dispatch_editor_mouse_input(state, px, py, hit)
            }
            AppAction::EditorCursorMoved { px, py, hit } => {
                self.dispatch_editor_cursor_moved(px, py, hit)
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
            AppAction::ToggleMindmapStylePanel => self.toggle_active_mindmap_style_panel(),
            AppAction::MindmapStylePanel(action) => {
                self.dispatch_mindmap_style_panel_action(action)
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

    fn toggle_active_mindmap_style_panel(&mut self) -> AppEffect {
        if self.active_plugin_name() != Some(ui::plugin::PLUGIN_MINDMAP) {
            return AppEffect::NONE;
        }
        let Some(mut tab) = self.active_tab_session_mut() else {
            return AppEffect::NONE;
        };

        tab.toggle_mindmap_style_panel();
        AppEffect::REDRAW
    }

    fn dispatch_mindmap_style_panel_action(
        &mut self,
        action: ui::core::widget::MindmapStylePanelAction,
    ) -> AppEffect {
        use ui::core::widget::MindmapStylePanelAction;

        match action {
            MindmapStylePanelAction::Close => {
                if self.active_plugin_name() != Some(ui::plugin::PLUGIN_MINDMAP) {
                    return AppEffect::NONE;
                }
                let Some(mut tab) = self.active_tab_session_mut() else {
                    return AppEffect::NONE;
                };
                if !tab.mindmap_style_panel().is_visible() {
                    return AppEffect::NONE;
                }
                tab.close_mindmap_style_panel();
                AppEffect::REDRAW
            }
            MindmapStylePanelAction::TogglePresets => {
                if self.active_plugin_name() != Some(ui::plugin::PLUGIN_MINDMAP) {
                    return AppEffect::NONE;
                }
                let Some(mut tab) = self.active_tab_session_mut() else {
                    return AppEffect::NONE;
                };
                if !tab.mindmap_style_panel().is_visible() {
                    return AppEffect::NONE;
                }
                tab.toggle_mindmap_style_presets();
                AppEffect::REDRAW
            }
            MindmapStylePanelAction::SelectTheme(theme_id) => {
                self.apply_active_mindmap_theme(theme_id)
            }
        }
    }

    fn apply_active_mindmap_theme(&mut self, theme_id: String) -> AppEffect {
        let edit_executed = {
            let Some(session) = self.active_tab_session_mut() else {
                return AppEffect::NONE;
            };
            if session.plugin_name() != ui::plugin::PLUGIN_MINDMAP {
                return AppEffect::NONE;
            }

            let source_generation = session.document.generation();
            let Some(plan) = session.plan_mindmap_theme(theme_id, source_generation) else {
                return AppEffect::NONE;
            };

            crate::edit_transaction::execute_edit_plan(plan, session.document, &[])
                .map(|outcome| outcome.edit_outcome.executed)
                .unwrap_or(false)
        };

        if !edit_executed {
            return AppEffect::NONE;
        }

        self.sync_plugin_state();
        AppEffect::REDRAW
    }

    pub(crate) fn dispatch_settings_view_action(
        &mut self,
        action: ui::settings_view::SettingsViewAction,
    ) -> AppEffect {
        let effect = match action {
            ui::settings_view::SettingsViewAction::SetThemeMode(mode) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetThemeMode(mode))
            }
            ui::settings_view::SettingsViewAction::SetFontFamily(family) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetFontFamily(family))
            }
            ui::settings_view::SettingsViewAction::SetFontSize(size) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetFontSize(size))
            }
            ui::settings_view::SettingsViewAction::SetLineHeightRatio(ratio) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetLineHeightRatio(ratio))
            }
            ui::settings_view::SettingsViewAction::SetWordWrap(enabled) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetWordWrap(enabled))
            }
            ui::settings_view::SettingsViewAction::SetShowLineNumbers(enabled) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetShowLineNumbers(enabled))
            }
            ui::settings_view::SettingsViewAction::SetTabWidth(width) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetTabWidth(width))
            }
            ui::settings_view::SettingsViewAction::SetViewMode(mode) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetViewMode(mode))
            }
            ui::settings_view::SettingsViewAction::SetShowStatusBar(enabled) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetShowStatusBar(enabled))
            }
            ui::settings_view::SettingsViewAction::RetryPersistence => AppEffect::PERSIST_SETTINGS,
        };
        self.refresh_settings_overlay();
        effect
    }

    fn dispatch_sync_settings_action(&mut self, action: SyncSettingsAction) -> AppEffect {
        self.dispatch_sync_settings_action_with_folder_picker(action, &mut || {
            rfd::FileDialog::new().pick_folder()
        })
    }

    fn dispatch_sync_settings_action_with_folder_picker(
        &mut self,
        action: SyncSettingsAction,
        pick_folder: &mut dyn FnMut() -> Option<std::path::PathBuf>,
    ) -> AppEffect {
        match action {
            SyncSettingsAction::TestConnection { endpoint, api_key } => {
                let endpoint = match parse_sync_endpoint(&endpoint) {
                    Ok(endpoint) => endpoint,
                    Err(message) => return self.report_sync_settings_error(message),
                };
                let api_key = api_key.expose().to_owned();
                let result = self
                    .sync_controller_mut()
                    .map(|controller| controller.test_connection(endpoint, api_key))
                    .unwrap_or(Err(crate::sync_controller::SyncControllerError::WorkerUnavailable));
                self.finish_sync_controller_action(result)
            }
            SyncSettingsAction::ConfigureConnection { endpoint, api_key } => {
                let endpoint = match parse_sync_endpoint(&endpoint) {
                    Ok(endpoint) => endpoint,
                    Err(message) => return self.report_sync_settings_error(message),
                };
                let api_key = api_key.expose().to_owned();
                let result = self
                    .sync_controller_mut()
                    .map(|controller| controller.configure_connection(endpoint, api_key))
                    .unwrap_or(Err(crate::sync_controller::SyncControllerError::WorkerUnavailable));
                self.finish_sync_controller_action(result)
            }
            SyncSettingsAction::PublishLibrary {
                remote_device_id,
                remote_name,
                remote_addresses,
            } => {
                let Some(root) = pick_folder() else {
                    return AppEffect::NONE;
                };
                let remote = match parse_sync_remote_device(
                    remote_device_id,
                    remote_name,
                    remote_addresses.join(","),
                ) {
                    Ok(remote) => remote,
                    Err(message) => return self.report_sync_settings_error(message),
                };
                let result = self
                    .sync_controller_mut()
                    .map(|controller| controller.publish_library(root, remote))
                    .unwrap_or(Err(crate::sync_controller::SyncControllerError::WorkerUnavailable));
                self.finish_sync_controller_action(result)
            }
            SyncSettingsAction::AcceptRemoteLibrary { pending_index } => {
                let folder_id = self
                    .sync_controller()
                    .and_then(|controller| {
                        controller.snapshot().pending_folders.get(pending_index).cloned()
                    })
                    .map(|pending| pending.folder_id);
                let Some(folder_id) = folder_id else {
                    return self.report_sync_settings_error(SYNC_PENDING_FOLDER_NOT_FOUND_NOTICE);
                };
                let Some(empty_root) = pick_folder() else {
                    return AppEffect::NONE;
                };
                let result = self
                    .sync_controller_mut()
                    .map(|controller| controller.accept_remote_library(folder_id, empty_root))
                    .unwrap_or(Err(crate::sync_controller::SyncControllerError::WorkerUnavailable));
                self.finish_sync_controller_action(result)
            }
            SyncSettingsAction::ScanLibrary { library_index } => self
                .dispatch_library_controller_action(library_index, |controller, library_id| {
                    controller.scan_library(library_id)
                }),
            SyncSettingsAction::SetLibraryPaused { library_index, paused } => self
                .dispatch_library_controller_action(
                    library_index,
                    move |controller, library_id| controller.pause_library(library_id, paused),
                ),
            SyncSettingsAction::RepairLibrary { library_index } => self
                .dispatch_library_controller_action(library_index, |controller, library_id| {
                    controller.repair_library(library_id)
                }),
            SyncSettingsAction::RemoveLibraryMapping { library_index } => self
                .dispatch_library_controller_action(library_index, |controller, library_id| {
                    controller.remove_library_mapping(library_id)
                }),
            SyncSettingsAction::UnregisterLibrary { library_index } => self
                .dispatch_library_controller_action(library_index, |controller, library_id| {
                    controller.unregister_library(library_id)
                }),
        }
    }

    fn dispatch_library_controller_action<F>(
        &mut self,
        library_index: usize,
        operation: F,
    ) -> AppEffect
    where
        F: FnOnce(
            &mut crate::sync_controller::SyncController,
            String,
        ) -> Result<
            crate::sync_controller::RequestId,
            crate::sync_controller::SyncControllerError,
        >,
    {
        let Some(library_id) = self
            .sync_controller()
            .and_then(|controller| controller.snapshot().libraries.get(library_index))
            .map(|library| library.library_id.clone())
        else {
            return self.report_sync_settings_error(SYNC_LIBRARY_NOT_FOUND_NOTICE);
        };
        let result = self
            .sync_controller_mut()
            .map(|controller| operation(controller, library_id))
            .unwrap_or(Err(crate::sync_controller::SyncControllerError::WorkerUnavailable));
        self.finish_sync_controller_action(result)
    }

    fn finish_sync_controller_action(
        &mut self,
        result: Result<
            crate::sync_controller::RequestId,
            crate::sync_controller::SyncControllerError,
        >,
    ) -> AppEffect {
        match result {
            Ok(_) => AppEffect::REDRAW,
            Err(error) => self.report_sync_settings_error(sync_controller_error_notice(&error)),
        }
    }

    fn report_sync_settings_error(&mut self, message: &'static str) -> AppEffect {
        if let Some(controller) = self.sync_controller_mut() {
            controller.push_local_error(message.to_owned());
        } else {
            eprintln!("[sync] {message}");
        }
        AppEffect::REDRAW
    }

    pub(crate) fn dispatch_tab_switch(&mut self, id: TabId) -> AppEffect {
        let Some(workspace_effect) = self.switch_editor_tab(id) else {
            return AppEffect::NONE;
        };

        let cancel_effect = self.cancel_canvas_drag();
        cancel_effect.merge(self.apply_workspace_effect(workspace_effect))
    }

    fn dispatch_canvas_scrollbar_action(
        &mut self,
        axis: ui::canvas::CanvasAxis,
        action: ui::scrollbar::ScrollbarAction,
    ) -> AppEffect {
        use ui::scrollbar::ScrollbarAction;

        let viewport_action = match action {
            ScrollbarAction::DragTo(position) => {
                CanvasViewportAction::SetAxisPosition { axis, position: position as f32 }
            }
            ScrollbarAction::PageUp => CanvasViewportAction::Page { axis, direction: -1.0 },
            ScrollbarAction::PageDown => CanvasViewportAction::Page { axis, direction: 1.0 },
            ScrollbarAction::StartDrag
            | ScrollbarAction::EndDrag
            | ScrollbarAction::HoverChanged(_) => return self.canvas_viewport_redraw_effect(),
        };

        self.dispatch_canvas_viewport_action(viewport_action)
    }

    fn dispatch_canvas_pinch(
        &mut self,
        delta: f64,
        screen_anchor: ui::canvas::CanvasPoint,
    ) -> AppEffect {
        if !delta.is_finite() {
            return AppEffect::NONE;
        }

        let factor = 1.0 + delta as f32;
        if !factor.is_finite() {
            return AppEffect::NONE;
        }

        self.dispatch_canvas_viewport_action(CanvasViewportAction::ZoomBy {
            factor: factor.max(MIN_CANVAS_PINCH_ZOOM_FACTOR),
            screen_anchor,
        })
    }

    fn canvas_viewport_redraw_effect(&self) -> AppEffect {
        if self.active_canvas_viewport_has_snapshot() { AppEffect::REDRAW } else { AppEffect::NONE }
    }

    fn dispatch_canvas_viewport_action(&mut self, action: CanvasViewportAction) -> AppEffect {
        let Some(mut tab) = self.active_tab_session_mut() else {
            return AppEffect::NONE;
        };
        if !tab.is_canvas() || !tab.has_canvas_viewport_snapshot() {
            return AppEffect::NONE;
        }

        tab.apply_canvas_viewport_action(action);
        AppEffect::REDRAW
    }

    fn active_canvas_viewport_has_snapshot(&self) -> bool {
        self.active_is_canvas() && self.active_has_canvas_viewport_snapshot()
    }

    /// Rebuild the theme from system theme and settings mode.
    /// Handle sidebar keyboard actions (TogglePin, Escape).
    /// Compute preview render offsets (shared by mouse input and cursor moved).
    pub(crate) fn preview_offsets(&self) -> (f32, f32) {
        let metrics = self.ui_metrics();
        let dpi = metrics.dpi;
        let preview_top_pad = 16.0 * dpi;
        let line_count = self.active_document_line_count();
        let gutter_left_margin = self.editor_left_margin(line_count);
        let content_top = self.content_top_offset();
        (gutter_left_margin, content_top + preview_top_pad)
    }

    /// Compute the plugin render bounds (the same Rect + viewport dimensions
    /// used by the main render pass). Used for the WYSIWYG two-phase hit-test
    /// inter-phase layout refresh mini-render.
    pub(crate) fn plugin_render_bounds(&self) -> ui::core::geom::Rect {
        let metrics = self.ui_metrics();
        let dpi = metrics.dpi;
        let editor_r = self.plugin_editor_rect();
        let toc_off = self.toc_left_offset();
        let content_top = self.content_top_offset();

        let is_canvas = self.active_is_canvas();
        if is_canvas {
            let offset_x = editor_r.x;
            let offset_y = content_top;
            let viewport_w = editor_r.w;
            let viewport_h = (editor_r.h - (content_top - editor_r.y)).max(MIN_PLUGIN_VIEWPORT_PX);
            return ui::core::geom::Rect::new(offset_x, offset_y, viewport_w, viewport_h);
        }

        let preview_top_pad = PREVIEW_TOP_PAD_LOGICAL * dpi;
        let preview_pad = PREVIEW_HORIZONTAL_PAD_LOGICAL * dpi;
        let reading_base_x = toc_off + preview_pad;
        let physical_w = (editor_r.w - reading_base_x - preview_pad).max(MIN_PLUGIN_VIEWPORT_PX);
        let max_reading_width = MAX_READING_WIDTH_LOGICAL * dpi;
        let viewport_w = physical_w.min(max_reading_width);
        let viewport_h = (editor_r.h - preview_top_pad).max(MIN_PLUGIN_VIEWPORT_PX);
        let offset_x = editor_r.x + reading_base_x + (physical_w - viewport_w) * 0.5;
        let offset_y = content_top + preview_top_pad;

        ui::core::geom::Rect::new(offset_x, offset_y, viewport_w, viewport_h)
    }

    fn plugin_editor_rect(&self) -> ui::core::geom::Rect {
        let cached = self.ui_shell.editor_rect();
        if cached.w > 0.0 && cached.h > 0.0 && !self.ui_shell.dock_is_dirty() {
            return cached;
        }

        let inputs = self.build_shell_inputs();
        projected_editor_rect(
            self.screen_width(),
            self.screen_height(),
            &inputs,
            self.ui_shell.mindmap_style_panel_thickness(),
        )
    }
}

#[cfg(test)]
pub(crate) mod canvas_drag_test_support {
    use crate::app::App;
    use crate::document_view::DocumentView;

    use std::cell::RefCell;
    use std::rc::Rc;
    use ui::plugin::{
        CanvasDragRequest, CanvasDragResponse, EditHitTarget, PluginMessage, PluginQuery,
        PluginResponse, ViewPlugin,
    };
    use winit::event::ElementState;

    #[derive(Default)]
    pub(crate) struct CanvasDragState {
        pub(crate) requests: Vec<CanvasDragRequest>,
    }

    struct CanvasDragPlugin {
        state: Rc<RefCell<CanvasDragState>>,
    }

    impl ViewPlugin for CanvasDragPlugin {
        fn name(&self) -> &str {
            "canvas_drag_lifecycle_test"
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
                PluginQuery::NeedsSourceUpdate(_) => PluginResponse::Bool(false),
                PluginQuery::HitTestEditTarget { .. } => {
                    PluginResponse::EditHitTarget(Some(EditHitTarget::SourceObject {
                        source_range: 1..2,
                    }))
                }
                PluginQuery::ContentHeight => PluginResponse::Float(1_000.0),
                _ => PluginResponse::None,
            }
        }

        fn handle_message(
            &mut self,
            _message: PluginMessage,
            _doc: &mut dyn core::document::DocViewMut,
        ) -> bool {
            false
        }

        fn handle_canvas_drag(
            &mut self,
            request: CanvasDragRequest,
            _doc: &dyn core::document::DocView,
        ) -> CanvasDragResponse {
            self.state.borrow_mut().requests.push(request);
            CanvasDragResponse::Ignore
        }
    }

    pub(crate) fn app_with_canvas_drag_tabs() -> (App, Rc<RefCell<CanvasDragState>>) {
        let state = Rc::new(RefCell::new(CanvasDragState::default()));
        let mut app = App::new(None);
        for text in ["abc", "def"] {
            let document = DocumentView::new(vec![text.to_string()], 80, 10.0);
            app.push_entry_for_test(document, Box::new(CanvasDragPlugin { state: state.clone() }));
        }
        app.switch_workspace_for_test(0);
        (app, state)
    }

    pub(crate) fn start_canvas_drag(app: &mut App) {
        let bounds = app.plugin_render_bounds();
        app.dispatch_editor_mouse_input(
            ElementState::Pressed,
            bounds.x + 4.0,
            bounds.y + 4.0,
            None,
        );
        app.dispatch_editor_cursor_moved(bounds.x + 20.0, bounds.y + 20.0, None);
    }

    pub(crate) fn cancel_request_count(state: &CanvasDragState) -> usize {
        state
            .requests
            .iter()
            .filter(|request| request.phase == ui::plugin::CanvasDragPhase::Cancel)
            .count()
    }

    pub(crate) fn document_texts(app: &App) -> Vec<String> {
        app.editor_tab_ids_in_order()
            .into_iter()
            .map(|tab_id| app.tab_session(tab_id).expect("tab session").full_text())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::canvas_drag_test_support::{
        app_with_canvas_drag_tabs, cancel_request_count, document_texts, start_canvas_drag,
    };
    use super::*;
    use crate::document_view::DocumentView;

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    fn has_direct_sync_controller_field_access(compact_source: &str) -> bool {
        let sync_controller_field = ["self.sync_", "controller"].concat();

        compact_source.match_indices(&sync_controller_field).any(|(index, _)| {
            let suffix = &compact_source[index + sync_controller_field.len()..];
            !suffix.starts_with("()") && !suffix.starts_with("_mut()")
        })
    }

    #[test]
    fn dispatch_routes_sync_controller_access_through_app_accessors() {
        let production_source = include_str!("app_dispatch.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("dispatch production source should precede tests");
        let compact_production_source = production_source.split_whitespace().collect::<String>();

        assert!(
            !has_direct_sync_controller_field_access(&compact_production_source),
            "dispatch must use App sync-controller accessors"
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

    #[derive(Clone, Copy)]
    enum MindmapThemePlanMode {
        Current(&'static str),
        Stale,
    }

    #[derive(Default)]
    struct MindmapStyleDispatchState {
        plan_queries: RefCell<Vec<(String, u32)>>,
        sync_queries: Cell<usize>,
    }

    struct MindmapStyleDispatchPlugin {
        name: &'static str,
        mode: MindmapThemePlanMode,
        state: Rc<MindmapStyleDispatchState>,
    }

    impl ui::plugin::ViewPlugin for MindmapStyleDispatchPlugin {
        fn name(&self) -> &str {
            self.name
        }

        fn render(
            &mut self,
            _doc: &dyn core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }

        fn query(
            &self,
            query: ui::plugin::PluginQuery,
            _doc: &dyn core::document::DocView,
        ) -> ui::plugin::PluginResponse {
            use ui::plugin::{EditPlan, EditTransaction, PluginQuery, PluginResponse};

            match query {
                PluginQuery::PlanMindmapTheme { theme_id, source_generation } => {
                    self.state
                        .plan_queries
                        .borrow_mut()
                        .push((theme_id.clone(), source_generation));
                    let plan = match self.mode {
                        MindmapThemePlanMode::Current(current) if theme_id == current => {
                            EditPlan::Consume
                        }
                        MindmapThemePlanMode::Current(_) => {
                            let replacement = format!("theme={theme_id}");
                            let cursor_after = replacement.len();
                            EditPlan::Apply(EditTransaction::replace(
                                source_generation,
                                0..4,
                                replacement,
                                cursor_after,
                            ))
                        }
                        MindmapThemePlanMode::Stale => {
                            let replacement = format!("theme={theme_id}");
                            let cursor_after = replacement.len();
                            EditPlan::Apply(EditTransaction::replace(
                                source_generation.wrapping_sub(1),
                                0..4,
                                replacement,
                                cursor_after,
                            ))
                        }
                    };
                    PluginResponse::EditPlan(plan)
                }
                PluginQuery::NeedsSourceUpdate(_) => {
                    self.state.sync_queries.set(self.state.sync_queries.get() + 1);
                    PluginResponse::Bool(false)
                }
                _ => PluginResponse::None,
            }
        }
    }

    fn app_with_mindmap_style_plugin(
        name: &'static str,
        mode: MindmapThemePlanMode,
    ) -> (App, Rc<MindmapStyleDispatchState>) {
        let state = Rc::new(MindmapStyleDispatchState::default());
        let plugin = MindmapStyleDispatchPlugin { name, mode, state: state.clone() };
        let mut app = App::new(None);
        let document = DocumentView::new(vec!["root".to_owned()], 80, 10.0);
        app.push_entry_for_test(document, Box::new(plugin));
        app.switch_workspace_for_test(0);
        (app, state)
    }

    fn reduce_mindmap_style_action(
        app: &mut App,
        action: ui::core::widget::MindmapStylePanelAction,
    ) -> AppEffect {
        app.reduce_action(AppAction::MindmapStylePanel(action), None)
    }

    #[test]
    fn mindmap_style_title_toggle_opens_expanded_and_closes_active_panel() {
        let (mut app, _) = app_with_mindmap_style_plugin(
            ui::plugin::PLUGIN_MINDMAP,
            MindmapThemePlanMode::Current("warm-night"),
        );

        assert_eq!(app.reduce_action(AppAction::ToggleMindmapStylePanel, None), AppEffect::REDRAW);
        assert_eq!(
            app.active_tab_session().expect("active mmap tab").mindmap_style_panel(),
            crate::tab::MindmapStylePanelSession::Open { presets_expanded: true }
        );

        assert_eq!(app.reduce_action(AppAction::ToggleMindmapStylePanel, None), AppEffect::REDRAW);
        assert_eq!(
            app.active_tab_session().expect("active mmap tab").mindmap_style_panel(),
            crate::tab::MindmapStylePanelSession::Closed
        );
    }

    #[test]
    fn mindmap_style_close_and_preset_toggle_change_only_open_active_session() {
        let (mut app, _) = app_with_mindmap_style_plugin(
            ui::plugin::PLUGIN_MINDMAP,
            MindmapThemePlanMode::Current("warm-night"),
        );
        let second_state = Rc::new(MindmapStyleDispatchState::default());
        let second_tab_id = app.push_entry_for_test(
            DocumentView::new(vec!["root".to_owned()], 80, 10.0),
            Box::new(MindmapStyleDispatchPlugin {
                name: ui::plugin::PLUGIN_MINDMAP,
                mode: MindmapThemePlanMode::Current("warm-night"),
                state: second_state,
            }),
        );
        let first_tab_id = app.editor_tab_id_at(0).expect("first tab id");
        app.tab_session_mut(first_tab_id).expect("first tab").toggle_mindmap_style_panel();
        app.tab_session_mut(second_tab_id).expect("second tab").toggle_mindmap_style_panel();
        app.switch_workspace_for_test(1);

        assert_eq!(
            reduce_mindmap_style_action(
                &mut app,
                ui::core::widget::MindmapStylePanelAction::TogglePresets,
            ),
            AppEffect::REDRAW
        );
        assert_eq!(
            app.active_tab_session().expect("second active tab").mindmap_style_panel(),
            crate::tab::MindmapStylePanelSession::Open { presets_expanded: false }
        );
        assert_eq!(
            reduce_mindmap_style_action(&mut app, ui::core::widget::MindmapStylePanelAction::Close,),
            AppEffect::REDRAW
        );
        assert_eq!(
            app.tab_session(first_tab_id).expect("first tab").mindmap_style_panel(),
            crate::tab::MindmapStylePanelSession::Open { presets_expanded: true }
        );
        assert_eq!(
            app.tab_session(second_tab_id).expect("second tab").mindmap_style_panel(),
            crate::tab::MindmapStylePanelSession::Closed
        );
        assert_eq!(
            reduce_mindmap_style_action(
                &mut app,
                ui::core::widget::MindmapStylePanelAction::TogglePresets,
            ),
            AppEffect::NONE
        );
    }

    #[test]
    fn mindmap_style_selection_executes_one_transaction_and_syncs_after_text_change() {
        let (mut app, state) = app_with_mindmap_style_plugin(
            ui::plugin::PLUGIN_MINDMAP,
            MindmapThemePlanMode::Current("warm-night"),
        );
        let generation_before =
            app.active_tab_session().expect("active mmap tab").document.generation();

        let effect = reduce_mindmap_style_action(
            &mut app,
            ui::core::widget::MindmapStylePanelAction::SelectTheme("tide".into()),
        );

        let entry = app.active_tab_session().expect("active mmap tab");
        assert_eq!(effect, AppEffect::REDRAW);
        assert_eq!(entry.document.full_text(), "theme=tide");
        assert_eq!(entry.document.generation(), generation_before + 1);
        assert!(entry.document.dirty);
        assert_eq!(state.plan_queries.borrow().as_slice(), &[("tide".into(), generation_before)]);
        assert_eq!(state.sync_queries.get(), 1);
    }

    #[test]
    fn mindmap_style_current_theme_is_consumed_without_document_change_or_sync() {
        let (mut app, state) = app_with_mindmap_style_plugin(
            ui::plugin::PLUGIN_MINDMAP,
            MindmapThemePlanMode::Current("tide"),
        );
        let generation_before =
            app.active_tab_session().expect("active mmap tab").document.generation();

        let effect = reduce_mindmap_style_action(
            &mut app,
            ui::core::widget::MindmapStylePanelAction::SelectTheme("tide".into()),
        );

        let entry = app.active_tab_session().expect("active mmap tab");
        assert_eq!(effect, AppEffect::NONE);
        assert_eq!(entry.document.full_text(), "root");
        assert_eq!(entry.document.generation(), generation_before);
        assert!(!entry.document.dirty);
        assert_eq!(state.sync_queries.get(), 0);
    }

    #[test]
    fn mindmap_style_stale_plan_and_non_mmap_plugin_leave_documents_unchanged() {
        let (mut stale_app, stale_state) =
            app_with_mindmap_style_plugin(ui::plugin::PLUGIN_MINDMAP, MindmapThemePlanMode::Stale);
        let stale_generation =
            stale_app.active_tab_session().expect("active stale mmap tab").document.generation();
        assert_eq!(
            reduce_mindmap_style_action(
                &mut stale_app,
                ui::core::widget::MindmapStylePanelAction::SelectTheme("tide".into()),
            ),
            AppEffect::NONE
        );
        let stale_entry = stale_app.active_tab_session().expect("active stale mmap tab");
        assert_eq!(stale_entry.document.full_text(), "root");
        assert_eq!(stale_entry.document.generation(), stale_generation);
        assert!(!stale_entry.document.dirty);
        assert_eq!(stale_state.sync_queries.get(), 0);

        let (mut editor_app, editor_state) = app_with_mindmap_style_plugin(
            ui::plugin::PLUGIN_EDITOR,
            MindmapThemePlanMode::Current("warm-night"),
        );
        assert_eq!(
            reduce_mindmap_style_action(
                &mut editor_app,
                ui::core::widget::MindmapStylePanelAction::SelectTheme("tide".into()),
            ),
            AppEffect::NONE
        );
        assert_eq!(
            editor_app.active_tab_session().expect("active editor tab").document.full_text(),
            "root"
        );
        assert!(editor_state.plan_queries.borrow().is_empty());
    }

    #[test]
    fn mindmap_style_selection_updates_only_the_active_mmap_tab() {
        let (mut app, first_state) = app_with_mindmap_style_plugin(
            ui::plugin::PLUGIN_MINDMAP,
            MindmapThemePlanMode::Current("warm-night"),
        );
        let second_state = Rc::new(MindmapStyleDispatchState::default());
        app.push_entry_for_test(
            DocumentView::new(vec!["root".to_owned()], 80, 10.0),
            Box::new(MindmapStyleDispatchPlugin {
                name: ui::plugin::PLUGIN_MINDMAP,
                mode: MindmapThemePlanMode::Current("warm-night"),
                state: second_state.clone(),
            }),
        );
        app.switch_workspace_for_test(1);

        assert_eq!(
            reduce_mindmap_style_action(
                &mut app,
                ui::core::widget::MindmapStylePanelAction::SelectTheme("tide".into()),
            ),
            AppEffect::REDRAW
        );
        assert_eq!(
            app.tab_session(app.editor_tab_id_at(0).expect("first tab id"))
                .expect("first tab")
                .full_text(),
            "root"
        );
        assert_eq!(
            app.tab_session(app.editor_tab_id_at(1).expect("second tab id"))
                .expect("second tab")
                .full_text(),
            "theme=tide"
        );
        assert!(first_state.plan_queries.borrow().is_empty());
        assert_eq!(second_state.plan_queries.borrow().len(), 1);
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
        let panel_thickness = ui::mindmap_style_panel::PANEL_WIDTH_LOGICAL;
        let with_panel = projected_editor_rect(1_200.0, 800.0, &inputs, panel_thickness);

        assert_eq!(without_panel, ui::core::geom::Rect::new(0.0, 0.0, 1_188.0, 776.0));
        assert_eq!(with_panel, ui::core::geom::Rect::new(0.0, 0.0, 908.0, 776.0));
        inputs.scrollbar_thickness = 0.0;
        assert_eq!(projected_editor_rect(1_200.0, 800.0, &inputs, panel_thickness).w, 920.0);
    }

    impl ui::plugin::ViewPlugin for WysiwygBoundsPlugin {
        fn handles_own_rendering(&self) -> bool {
            true
        }
        fn name(&self) -> &str {
            "wysiwyg_bounds"
        }
        fn render(
            &mut self,
            _doc: &dyn core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }
    }

    struct CanvasViewportRouterPlugin;

    impl ui::plugin::ViewPlugin for CanvasViewportRouterPlugin {
        fn name(&self) -> &str {
            "canvas_viewport_router"
        }

        fn render(
            &mut self,
            _doc: &dyn core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }

        fn is_canvas(&self) -> bool {
            true
        }
    }

    fn app_with_prepared_canvas_viewport() -> App {
        let mut app = App::new(None);
        let document = DocumentView::new(vec!["canvas".to_string()], 80, 10.0);
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
        let document = DocumentView::new(vec!["canvas".to_string()], 80, 10.0);
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
        let doc = DocumentView::new(vec!["hello world".to_string()], 80, 10.0);
        app.push_entry_for_test(doc, Box::new(WysiwygBoundsPlugin));
        app.switch_workspace_for_test(0);

        assert_eq!(app.ui_shell.editor_rect(), ui::core::geom::Rect::ZERO);

        let bounds = app.plugin_render_bounds();

        assert!(
            bounds.w > MIN_PLUGIN_VIEWPORT_PX
                && bounds.x > app.ui_shell.sidebar_editor_left_offset(),
            "initial WYSIWYG render bounds should use current screen width before shell layout, got {bounds:?}"
        );
    }

    #[test]
    fn wysiwyg_plugin_bounds_account_for_sidebar_during_first_shell_frame() {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        let doc = DocumentView::new(vec!["hello world".to_string()], 80, 10.0);
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
            bounds.x > app.ui_shell.sidebar_editor_left_offset(),
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
        assert_eq!(document_texts(&app), ["abc", "def"]);

        app.dispatch_tab_switch(app.editor_tab_id_at(0).unwrap());

        assert_eq!(cancel_request_count(&state.borrow()), 1);
    }

    #[test]
    fn settings_actions_apply_values_and_return_required_effects() {
        let mut app = App::new(None);

        assert!(
            app.dispatch_settings_view_action(ui::settings_view::SettingsViewAction::SetFontSize(
                18.0,
            ))
            .reshape
        );
        assert_eq!(app.settings.font_size, 18.0);

        assert!(
            app.dispatch_settings_view_action(ui::settings_view::SettingsViewAction::SetThemeMode(
                ui::settings::ThemeMode::Light,
            ),)
                .redraw
        );
        assert_eq!(app.settings.theme_mode, ui::settings::ThemeMode::Light);

        assert!(
            app.dispatch_settings_view_action(ui::settings_view::SettingsViewAction::SetViewMode(
                ui::view_mode::ViewMode::Tabs,
            ))
            .sync_window_chrome
        );
        assert_eq!(app.settings.view_mode, ui::view_mode::ViewMode::Tabs);
    }

    #[test]
    fn sync_settings_action_reaches_existing_controller_validation() {
        let mut app = App::new(None);
        let sync_action = SyncSettingsAction::TestConnection {
            endpoint: "https://example.com".to_owned(),
            api_key: ui::core::widget::SensitiveText::new("secret".to_owned()),
        };
        let debug = format!("{sync_action:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret"));

        let effect = app.reduce_action(AppAction::Sync(sync_action), None);
        assert!(effect.redraw);
    }

    #[test]
    fn publish_library_prompts_for_folder_before_validating_remote_fields() {
        let mut app = App::new(None);
        let mut folder_picker_call_count = 0;

        let effect = app.dispatch_sync_settings_action_with_folder_picker(
            SyncSettingsAction::PublishLibrary {
                remote_device_id: String::new(),
                remote_name: String::new(),
                remote_addresses: Vec::new(),
            },
            &mut || {
                folder_picker_call_count += 1;
                None
            },
        );

        assert_eq!(folder_picker_call_count, 1);
        assert_eq!(effect, AppEffect::NONE);
    }

    #[test]
    fn sidebar_settings_action_opens_modal_settings_view() {
        let mut app = App::new(None);

        let effect = app.reduce_action(AppAction::OpenSidebarSettingsMenu, None);

        assert!(effect.redraw);
        assert_eq!(app.ui_shell.overlays_count(), 1);
        assert!(app.ui_shell.active_overlay_is_modal());
    }

    #[test]
    fn sync_remote_input_parser_accepts_static_addresses_and_dynamic_placeholder() {
        let remote = parse_sync_remote_device(
            "ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG".to_owned(),
            "远端设备".to_owned(),
            " tcp://sync.example.com:22000, dynamic ".to_owned(),
        )
        .expect("valid sync remote input should parse");

        assert_eq!(
            remote.device_id.as_str(),
            "ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG"
        );
        assert_eq!(remote.name, "远端设备");
        assert_eq!(remote.addresses.len(), 1);
        assert_eq!(remote.addresses[0].as_str(), "tcp://sync.example.com:22000");
    }

    #[test]
    fn sync_remote_input_parser_rejects_invalid_device_or_address() {
        assert!(
            parse_sync_remote_device(
                "invalid".to_owned(),
                "远端设备".to_owned(),
                "dynamic".to_owned(),
            )
            .is_err()
        );
        assert!(
            parse_sync_remote_device(
                "ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG".to_owned(),
                "远端设备".to_owned(),
                "https://sync.example.com".to_owned(),
            )
            .is_err()
        );
    }
}
