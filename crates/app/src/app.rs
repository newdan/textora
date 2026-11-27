use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use appkit_shell::editor_runtime::{EditorFocus, EditorInputContext, EditorRuntime};
use appkit_shell::{EventPump, ProductHost, ProductWakeHandle};

use crate::native_menu::NativeMenu;
use crate::ui_shell::UiShell;

mod workspace_services;
pub(crate) use workspace_services::WorkspaceServices;
mod settings_ui_state;
pub(crate) use settings_ui_state::{SettingsPersistenceState, SettingsUiState};
mod platform_state;
pub(crate) use platform_state::{PlatformState, PreparedFontSystem};

// Font size and line height are now in Settings

/// 计算光标当前是否可见，以及下一次切换的时间点。
pub(crate) fn compute_cursor_phase(cursor_blink_instant: Instant) -> (bool, Instant) {
    let elapsed_ms = cursor_blink_instant.elapsed().as_millis() as u64;
    let period_ms: u64 = 500;
    let phase_in_period = elapsed_ms % (period_ms * 2);

    let currently_visible = phase_in_period < period_ms;
    let next_transition_ms = if currently_visible {
        period_ms - phase_in_period
    } else {
        period_ms * 2 - phase_in_period
    };

    // +5ms 容差，避免 WaitUntil 精度不足导致 phase 未变就被唤醒
    let next_deadline = Instant::now() + Duration::from_millis(next_transition_ms + 5);

    (currently_visible, next_deadline)
}

/// Textora product and editor composition root.
pub struct App {
    pub(crate) paths: crate::product_paths::ProductPaths,
    pub(crate) settings_ui: SettingsUiState,
    pub(crate) product: crate::textora_product::TextoraProduct,
    pub(crate) action_pump: EventPump<crate::actions::AppAction>,
    pub(crate) editor_runtime: EditorRuntime,
    pub(crate) workspace_services: WorkspaceServices,
    pub(crate) ui_shell: UiShell,
    pub(crate) platform: PlatformState,
}

impl App {
    pub(crate) fn editor_input_context(&self) -> EditorInputContext {
        let editor_is_focused = self.ui_shell.keyboard_focus()
            == crate::ui_shell::KeyboardFocusTarget::Editor
            && self.editor_runtime.window_focused();
        EditorInputContext {
            focus: if editor_is_focused { EditorFocus::Active } else { EditorFocus::Inactive },
            modal_blocked: self.ui_shell.active_overlay_is_modal(),
        }
    }

    pub fn open_document_sender(&self) -> crate::OpenDocumentSender {
        self.product.open_document_sender()
    }

    pub(crate) fn native_menu(&self) -> Option<&NativeMenu> {
        self.product.native_menu()
    }

    pub(crate) fn set_native_menu(&mut self, native_menu: NativeMenu) {
        self.product.set_native_menu(native_menu);
    }

    #[cfg(feature = "sync")]
    pub(crate) fn sync_controller(&self) -> Option<&crate::sync_controller::SyncController> {
        self.product.sync_controller()
    }

    #[cfg(feature = "sync")]
    pub(crate) fn sync_controller_mut(
        &mut self,
    ) -> Option<&mut crate::sync_controller::SyncController> {
        self.product.sync_controller_mut()
    }

    pub(crate) fn snapshot_popup_tab_ids(&mut self) {
        let ids = self.editor_tab_ids_in_order();
        self.ui_shell.set_popup_tab_ids(ids);
    }

    pub(crate) fn clear_popup_tab_id_snapshot(&mut self) {
        self.ui_shell.clear_popup_tab_ids();
    }

    pub(crate) fn popup_tab_id_for_index(
        &self,
        index: usize,
    ) -> Option<appkit_core::tab_set::types::TabId> {
        self.ui_shell.popup_tab_id(index).or_else(|| self.editor_tab_id_at(index))
    }

    /// Register the event-loop proxy before background work is started.
    pub fn set_event_loop_proxy(
        &mut self,
        event_loop_proxy: winit::event_loop::EventLoopProxy<crate::app_event::AppEvent>,
    ) {
        self.platform.register_event_loop_proxy(event_loop_proxy.clone());
        let file_safety_proxy = event_loop_proxy.clone();
        if !self.editor_runtime.file_safety_worker_started() {
            self.editor_runtime.start_file_safety_worker(move || {
                let _ = file_safety_proxy
                    .send_event(crate::app_event::AppEvent::FileSafetyResultsReady);
            });
        }
    }

    pub(crate) fn start_background_services(&mut self) {
        let Some(event_loop_proxy) = self.platform.event_loop_proxy().cloned() else { return };

        if self.workspace_services.monitor().is_none() {
            let monitor_proxy = event_loop_proxy.clone();
            match crate::library_file_monitor::LibraryFileMonitor::spawn(move || {
                let _ =
                    monitor_proxy.send_event(crate::app_event::AppEvent::FileSafetyResultsReady);
            }) {
                Ok(monitor) => {
                    self.workspace_services.install_monitor(monitor);
                    self.refresh_file_monitor_roots();
                }
                Err(error) => eprintln!("[file-monitor] failed to start: {error}"),
            }
        }

        ProductHost::start_background_services(
            &mut self.product,
            ProductWakeHandle::new(event_loop_proxy),
        );
    }

    fn file_monitor_roots(&self) -> Vec<PathBuf> {
        self.editor_runtime
            .document_summaries()
            .iter()
            .filter_map(|summary| summary.path.as_deref())
            .filter_map(|path| path.parent())
            .map(Path::to_owned)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub(crate) fn refresh_file_monitor_roots(&self) {
        let Some(monitor) = self.workspace_services.monitor() else { return };
        if let Err(error) = monitor.replace_roots(self.file_monitor_roots()) {
            eprintln!("[file-monitor] failed to refresh roots: {error}");
        }
    }

    /// Top offset for editor content positioning.
    /// Returns the combined height of the top chrome (tab bar or title bar)
    /// plus the search bar when visible, so text rendering starts below them.
    pub(crate) fn content_top_offset(&self) -> f32 {
        let metrics = self.ui_metrics();
        // Search bar height (common to both modes)
        let search_visible =
            self.active_tab_session().is_some_and(|tab| tab.search_state().panel_visible);
        let search_h =
            if search_visible { ui::search_bar::SEARCH_BAR_HEIGHT * metrics.dpi } else { 0.0 };

        // Sidebar mode: title bar + search bar
        if matches!(self.editor_runtime.settings().view_mode, ui::view_mode::ViewMode::Sidebar) {
            return ui::title_bar::title_bar_height(metrics.dpi) + search_h;
        }

        // Tabs mode keeps the shared title bar above the optional tab strip.
        let tbh = self.current_tab_bar_height_with_metrics(&metrics);
        ui::title_bar::title_bar_height(metrics.dpi) + tbh + search_h
    }

    pub(crate) fn visible_rows(&self, screen_height: f32) -> usize {
        self.visible_height_lines(screen_height).floor() as usize
    }

    pub(crate) fn visible_height_lines(&self, screen_height: f32) -> f64 {
        let metrics = self.ui_metrics();
        let status_h = if self.editor_runtime.settings().show_status_bar {
            metrics.status_bar_height
        } else {
            0.0
        };
        ((screen_height - status_h - self.content_top_offset()) / metrics.line_height).max(1.0)
            as f64
    }

    /// Compute explicit `ViewportDimensions` from instance settings and screen height.
    pub(crate) fn viewport_dimensions(
        &self,
        screen_height: f32,
    ) -> crate::workspace_tab_factory::ViewportDimensions {
        crate::workspace_tab_factory::ViewportDimensions {
            visible_rows: self.visible_rows(screen_height),
            viewport_height: self.visible_height_lines(screen_height),
        }
    }

    pub(crate) fn screen_width(&self) -> f32 {
        self.editor_runtime.surface_size().map_or(800.0, |(width, _)| width as f32)
    }

    pub(crate) fn screen_height(&self) -> f32 {
        self.editor_runtime.surface_size().map_or(600.0, |(_, height)| height as f32)
    }

    pub(crate) fn viewport_content_width(
        &self,
        document: &appkit_core::document::DocumentModel,
    ) -> f32 {
        let metrics = self.ui_metrics();
        let left_margin = self.toc_left_offset()
            + self.editor_left_margin_with_metrics(document.line_count(), &metrics);
        let editor_rect = self.ui_shell.editor_rect();
        let editor_right = editor_rect.x + editor_rect.w;
        let physical_w = editor_right - left_margin;
        const NO_WRAP_SENTINEL: f32 = 1_000_000.0;
        if self.editor_runtime.settings().word_wrap {
            physical_w.max(1.0)
        } else {
            NO_WRAP_SENTINEL
        }
    }

    pub(crate) fn current_tab_bar_height(&self) -> f32 {
        self.current_tab_bar_height_with_metrics(&self.ui_metrics())
    }

    pub(crate) fn current_tab_bar_height_with_metrics(
        &self,
        metrics: &ui::settings::UiMetrics,
    ) -> f32 {
        if self.editor_runtime.settings().view_mode == ui::view_mode::ViewMode::Tabs
            && self.editor_tab_count() > 1
        {
            ui::tab_bar::tab_bar_height(metrics.dpi)
        } else {
            0.0
        }
    }

    /// Return UI metrics derived from current settings.
    /// Settings holds logical (pre-DPI-scale) values; this multiplies by
    /// the current scale factor to produce physical pixel values.
    pub(crate) fn ui_metrics(&self) -> ui::settings::UiMetrics {
        ui::settings::UiMetrics::from_settings(
            self.editor_runtime.settings(),
            self.editor_runtime.scale_factor() as f32,
        )
    }

    /// Store the new scale factor. Does NOT modify dimensional settings fields.
    pub(crate) fn update_scale_factor(&mut self, scale_factor: f64) {
        let normalized_scale_factor =
            if scale_factor.is_finite() && scale_factor > 0.0 { scale_factor } else { 1.0 };
        self.editor_runtime.set_scale_factor(normalized_scale_factor);
    }

    /// Logical (pre-DPI-scale) font size, for persistence.
    pub(crate) fn persisted_font_size(&self) -> f32 {
        self.editor_runtime.settings().font_size
    }

    pub(crate) fn persist_editor_settings(&self) -> std::io::Result<()> {
        crate::settings_io::save_editor_settings(
            &self.paths.settings_file,
            self.editor_runtime.settings(),
        )
    }

    pub(crate) fn sync_window_chrome(&self) {
        let Some(window) = self.editor_runtime.window() else {
            return;
        };
        crate::sys::macos_titlebar::enable_full_size_content(window);
    }

    pub(crate) fn apply_effect(&mut self, effect: crate::app_effect::AppEffect) {
        use crate::app_effect::AppEffectStep;

        for step in effect.steps() {
            match step {
                AppEffectStep::Reshape => self.invalidate_reshape(),
                AppEffectStep::SyncWindowChrome => self.sync_window_chrome(),
                AppEffectStep::UpdateTitle => self.update_window_title(),
                AppEffectStep::PersistSettings => {
                    self.record_settings_persistence_result(self.persist_editor_settings());
                }
                AppEffectStep::PersistWorkspace => self.persist_workspace_state(),
                AppEffectStep::Redraw => {
                    self.platform.schedule_redraw();
                }
            }
        }
    }
}

#[cfg(test)]
mod product_ownership_boundary_tests {
    #[test]
    fn app_tab_contains_the_semantic_model_adapter() {
        let source = include_str!("app_tab.rs");
        for method_name in [
            "install_editor_tab",
            "activate_editor_tab",
            "append_editor_tab",
            "editor_close_decision",
            "discard_editor_tab",
        ] {
            assert!(source.contains(&format!("fn {method_name}")));
        }
    }

    #[test]
    fn app_does_not_declare_sync_controller_field() {
        let app_source = include_str!("app.rs");
        let legacy_field = ["pub(crate) sync_", "controller:"].concat();

        assert!(
            !app_source.contains(&legacy_field),
            "App must delegate sync-controller ownership to TextoraProduct"
        );
    }

    #[test]
    fn app_does_not_declare_native_menu_field() {
        let app_source = include_str!("app.rs");
        let legacy_field = ["pub(crate) native_", "menu:"].concat();

        assert!(
            !app_source.contains(&legacy_field),
            "App must delegate native-menu ownership to TextoraProduct"
        );
    }
}

#[cfg(test)]
mod open_document_sender_tests {
    use std::path::PathBuf;

    use appkit_shell::ProductHost;

    use super::App;

    #[test]
    fn public_open_document_sender_routes_paths_to_product_inbox() {
        let mut app = App::new(None);
        let open_document_paths = vec![PathBuf::from("/tmp/from-application-open-urls.md")];

        app.open_document_sender()
            .send(open_document_paths.clone())
            .expect("new application owns the product open-document receiver");
        let _ = ProductHost::drain_product_events(&mut app.product);

        assert_eq!(app.product.drain_open_documents(), open_document_paths);
    }
}

#[cfg(test)]
mod background_startup_boundary_tests {
    #[test]
    fn event_loop_proxy_registration_does_not_start_deferred_services() {
        let production_source = include_str!("app.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("application production source should precede tests");
        let registration_start = production_source
            .find("pub fn set_event_loop_proxy")
            .expect("event-loop proxy registration should exist");
        let deferred_start = production_source
            .find("pub(crate) fn start_background_services")
            .expect("deferred background startup should exist");
        let registration_source = &production_source[registration_start..deferred_start];

        assert!(!registration_source.contains("LibraryFileMonitor::spawn"));
        assert!(!registration_source.contains("SyncController::new_default"));
    }

    #[test]
    fn deferred_startup_delegates_sync_lifecycle_to_product_host() {
        let production_source = include_str!("app.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("application production source should precede tests");
        let compact_production_source = production_source.split_whitespace().collect::<String>();
        let product_start = [
            "ProductHost::start_background_services(&mutself.product,",
            "ProductWakeHandle::new(event_loop_proxy),);",
        ]
        .concat();
        let legacy_sync_constructor = ["SyncController::new_", "default"].concat();

        assert!(compact_production_source.contains(&product_start));
        assert!(!compact_production_source.contains(&legacy_sync_constructor));
    }
}

#[cfg(test)]
impl App {
    #[cfg(feature = "sync")]
    pub(crate) fn set_sync_controller(
        &mut self,
        controller: crate::sync_controller::SyncController,
    ) {
        self.product.set_sync_controller(controller);
    }

    pub(crate) fn switch_workspace_for_test(&mut self, index: usize) {
        let tab_id = self.editor_tab_id_at(index).expect("test tab index should be valid");
        let _ = self.activate_editor_tab(tab_id);
    }
}

#[cfg(test)]
mod geometry_metrics_tests {
    use super::App;
    use crate::document_fixture::DocumentFixture;
    use crate::plugins::editor::EditorPlugin;

    #[test]
    fn app_geometry_uses_metrics_snapshot() {
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentFixture::new(vec!["first".into()], 80, 10.0),
            Box::new(EditorPlugin::new()),
        );
        app.push_entry_for_test(
            DocumentFixture::new(vec!["second".into()], 80, 10.0),
            Box::new(EditorPlugin::new()),
        );
        app.switch_workspace_for_test(0);
        app.update_scale_factor(2.0);
        app.editor_runtime
            .edit_settings(|settings| settings.view_mode = ui::view_mode::ViewMode::Tabs);

        let metrics = app.ui_metrics();
        // Geometry methods should derive from metrics, not settings directly.
        // Verify that current_tab_bar_height() matches what metrics.dpi would produce.
        assert_eq!(app.current_tab_bar_height(), ui::tab_bar::tab_bar_height(metrics.dpi));
        // editor_left_margin depends on line count; with 1 line, gutter is small
        // so content_left_margin dominates
        let lm = app.editor_left_margin(1);
        assert!(lm >= metrics.content_left_margin);
    }
}

#[cfg(test)]
mod file_monitor_root_tests {
    use super::App;
    use crate::document_fixture::DocumentFixture;
    use crate::plugins::editor::EditorPlugin;

    use std::path::PathBuf;

    #[test]
    fn file_monitor_roots_cover_all_open_file_parents_once() {
        let mut app = App::new(None);
        let mut first = DocumentFixture::new(vec!["first".into()], 10, 10.0);
        first.file_path = Some(PathBuf::from("/library/notes/first.md"));
        let mut second = DocumentFixture::new(vec!["second".into()], 10, 10.0);
        second.file_path = Some(PathBuf::from("/library/notes/second.md"));
        let mut third = DocumentFixture::new(vec!["third".into()], 10, 10.0);
        third.file_path = Some(PathBuf::from("/library/archive/third.md"));
        app.push_entry_for_test(first, Box::new(EditorPlugin::new()));
        app.push_entry_for_test(second, Box::new(EditorPlugin::new()));
        app.push_entry_for_test(third, Box::new(EditorPlugin::new()));

        assert_eq!(
            app.file_monitor_roots(),
            vec![PathBuf::from("/library/archive"), PathBuf::from("/library/notes")]
        );
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod app_tests;

#[cfg(test)]
#[path = "table_creation_acceptance_tests.rs"]
mod table_creation_acceptance_tests;

#[cfg(test)]
#[path = "table_navigation_input_tests.rs"]
mod table_navigation_input_tests;

#[cfg(test)]
#[path = "table_picker_ui_path_tests.rs"]
mod table_picker_ui_path_tests;

#[cfg(test)]
#[path = "settings_boundary_tests.rs"]
mod settings_boundary_tests;

#[cfg(test)]
mod edit_reset_tests {
    use appkit_shell::cursor_motion::CursorRenderState;

    #[test]
    fn cursor_reset_does_not_own_reshape_generation() {
        let mut state = CursorRenderState::new();
        state.sticky_x_dirty = false;
        let before = state.cursor_blink_instant;

        state.reset_after_edit();

        assert!(state.sticky_x_dirty);
        assert!(state.cursor_blink_instant >= before);
    }
}
