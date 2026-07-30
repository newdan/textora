use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use appkit_shell::{ProductHost, ProductWakeHandle};
use winit::window::Window;

pub(crate) use crate::render_state::{GpuState, TextState};

use crate::file_history::FileHistory;
use crate::frame_cache::FrameCache;
use crate::mouse::MouseState;
use crate::native_menu::NativeMenu;
use crate::reshape_worker::ReshapeWorker;
use crate::tab_runtime::TabRuntimeStore;
use crate::ui_shell::UiShell;
use crate::workspace::Workspace;
use ui::theme::Theme;

#[allow(unused_imports)]
use crate::commands::execute_edit_command_v2;
#[allow(unused_imports)]
use crate::input::EditCommand;
/// Per-visual-line data for hit-testing, selection rendering, and cursor movement.
#[allow(unused_imports)]
use render::GlyphVertex;
#[allow(unused_imports)]
use ui::render_geom::{AdvanceCacheEntry, compute_selection_highlight_quads};

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

/// Only reset cursor state after an edit; reshape is handled by AppEffect::RESHAPE.
pub(crate) fn reset_cursor_after_edit(
    cursor_render_state: &mut crate::cursor_motion::CursorRenderState,
) {
    cursor_render_state.sticky_x_dirty = true;
    cursor_render_state.cursor_blink_instant = Instant::now();
}

pub(crate) fn reset_after_edit(
    generation: &mut u64,
    pending_reshapes: &mut HashSet<usize>,
    reshape_worker: &Option<ReshapeWorker>,
    cursor_render_state: &mut crate::cursor_motion::CursorRenderState,
) {
    *generation += 1;
    pending_reshapes.clear();
    if let Some(w) = reshape_worker {
        w.cancel_before(*generation);
    }
    cursor_render_state.sticky_x_dirty = true;
    cursor_render_state.cursor_blink_instant = Instant::now();
}

/// The main application state.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SettingsPersistenceState {
    Saved,
    SaveFailed { message: String },
}

pub struct App {
    pub(crate) window: Option<Arc<Window>>,
    pub(crate) gpu: Option<GpuState>,
    pub(crate) text: Option<TextState>,
    pub(crate) file_path: Option<PathBuf>,
    pub(crate) paths: crate::product_paths::ProductPaths,
    pub(crate) settings: ui::settings::Settings,
    pub(crate) settings_persistence: SettingsPersistenceState,
    pub(crate) current_theme: Theme,
    pub(crate) theme_registry: ui::theme::ThemeRegistry,
    pub(crate) active_theme_pair: ui::theme::ActiveThemePair,
    pub(crate) theme_load_report: crate::theme_loader::ThemeLoadReport,
    pub(crate) product: crate::textora_product::TextoraProduct,
    pub(crate) workspace: Workspace,
    pub(crate) tab_runtime_store: TabRuntimeStore,
    pub(crate) popup_tab_id_snapshot: Vec<appkit_core::workspace::types::TabId>,
    pub(crate) workspace_store: crate::workspace_store::WorkspaceStore,
    pub(crate) ui_shell: UiShell,
    pub(crate) file_history: FileHistory,
    pub(crate) file_safety_worker: Option<crate::file_safety::FileSafetyWorker>,
    pub(crate) library_file_monitor: Option<crate::library_file_monitor::LibraryFileMonitor>,
    pub(crate) file_safety_notices: Vec<crate::file_safety::FileSafetyNotice>,
    pub(crate) file_safety_tracked: HashSet<PathBuf>,
    pub(crate) file_safety_pending: HashSet<PathBuf>,
    pub(crate) file_safety_next_request_id: u64,
    pub(crate) file_safety_next_check: Instant,
    pub(crate) scale_factor: f64,
    pub(crate) running: bool,
    pub(crate) needs_redraw: bool,
    pub(crate) sidebar_animating: bool,
    /// Tab bar smooth-scroll animation.
    pub(crate) tab_scroll: crate::smooth_scroll::SmoothScroll,
    pub(crate) modifiers: winit::keyboard::ModifiersState,
    /// Mouse state for click/drag handling.
    pub(crate) mouse: MouseState,
    /// Per-frame rendering cache (advance cache, cluster pool, first/last line).
    pub(crate) frame_cache: FrameCache,
    /// 上次滚轮事件的时间，用于快速滚动时不渲染、停手后再渲染。
    pub(crate) last_scroll_time: Instant,
    pub(crate) reshape_worker: Option<ReshapeWorker>,
    /// Shared FontSystem, created once and passed to worker + TextState.
    pub(crate) shared_font_system: Option<Arc<Mutex<shaping::FontSystem>>>,
    pub(crate) reshape_generation: u64,
    /// Track in-flight reshape submissions to prevent duplicates.
    pub(crate) pending_reshapes: HashSet<usize>,
    /// Skip next submit_reshape_ahead after init_display_map full build.
    pub(crate) skip_reshape_submit: bool,
    /// Last anchor doc_line we submitted reshape ahead for (debounce rapid scroll).
    pub(crate) last_reshape_anchor: usize,
    /// Timestamp of last render() call for frame-interval measurement.
    pub(crate) last_render_time: std::time::Instant,
    pub(crate) last_rr_time: std::time::Instant,
    /// Frame counter for periodic perf logging (debug only).
    pub(crate) render_frame_count: u32,
    /// Pending resize event (throttled to ~16ms / 60fps).
    pub(crate) pending_resize: Option<winit::dpi::PhysicalSize<u32>>,
    /// Timestamp of last handled resize (for 16ms throttle).
    pub(crate) last_resize_handled: Instant,
    /// 上一帧的光标可见状态，用于 about_to_wait 检测 phase 变化
    pub(crate) last_cursor_visible: bool,
    /// 窗口是否处于激活/聚焦状态
    pub(crate) window_focused: bool,
    /// 事件循环代理，用于后台线程唤醒主线程。
    pub(crate) event_loop_proxy:
        Option<winit::event_loop::EventLoopProxy<crate::app_event::AppEvent>>,
    /// IME 预编辑文本（正在组合中、尚未确认的拼音/字母）
    pub(crate) preedit_text: String,
    /// IME 预编辑光标位置 (start_byte, end_byte)，用于下划线高亮
    pub(crate) preedit_cursor: Option<(usize, usize)>,
    /// IME 预编辑文字的总像素宽度（每帧在 shape_visible_lines 前计算）
    pub(crate) preedit_advance_px: f32,
    /// WYSIWYG 模式下，上下移动时保持的首选 X 像素位置（sticky column）。
    pub(crate) wysiwyg_preferred_x: Option<f32>,
    /// 防止 WYSIWYG 拦截重入（augmented enter/backspace 递归时跳过拦截）。
    pub(crate) wysiwyg_recursing: bool,
    /// 首帧是否已 present（用于窗口延迟显示，避免启动白闪）。
    pub(crate) first_frame_presented: bool,
    /// 应用状态构造开始时刻，用于记录首帧可见的端到端耗时。
    pub(crate) startup_started_at: Instant,
}

impl App {
    pub fn open_document_sender(&self) -> crate::OpenDocumentSender {
        self.product.open_document_sender()
    }

    pub(crate) fn native_menu(&self) -> Option<&NativeMenu> {
        self.product.native_menu()
    }

    pub(crate) fn set_native_menu(&mut self, native_menu: NativeMenu) {
        self.product.set_native_menu(native_menu);
    }

    pub(crate) fn sync_controller(&self) -> Option<&crate::sync_controller::SyncController> {
        self.product.sync_controller()
    }

    pub(crate) fn sync_controller_mut(
        &mut self,
    ) -> Option<&mut crate::sync_controller::SyncController> {
        self.product.sync_controller_mut()
    }

    pub(crate) fn set_sync_controller(
        &mut self,
        controller: crate::sync_controller::SyncController,
    ) {
        self.product.set_sync_controller(controller);
    }

    pub(crate) fn take_sync_controller(
        &mut self,
    ) -> Option<crate::sync_controller::SyncController> {
        self.product.take_sync_controller()
    }

    pub(crate) fn snapshot_popup_tab_ids(&mut self) {
        self.popup_tab_id_snapshot =
            (0..self.workspace.len()).filter_map(|index| self.workspace.tab_id_at(index)).collect();
    }

    pub(crate) fn clear_popup_tab_id_snapshot(&mut self) {
        self.popup_tab_id_snapshot.clear();
    }

    pub(crate) fn popup_tab_id_for_index(
        &self,
        index: usize,
    ) -> Option<appkit_core::workspace::types::TabId> {
        self.popup_tab_id_snapshot.get(index).copied().or_else(|| self.workspace.tab_id_at(index))
    }

    /// Register the event-loop proxy before background work is started.
    pub fn set_event_loop_proxy(
        &mut self,
        event_loop_proxy: winit::event_loop::EventLoopProxy<crate::app_event::AppEvent>,
    ) {
        self.event_loop_proxy = Some(event_loop_proxy.clone());
        if self.file_safety_worker.is_none() {
            let file_safety_proxy = event_loop_proxy.clone();
            self.file_safety_worker =
                Some(crate::file_safety::FileSafetyWorker::spawn(move || {
                    let _ = file_safety_proxy
                        .send_event(crate::app_event::AppEvent::FileSafetyResultsReady);
                }));
        }
    }

    pub(crate) fn start_background_services(&mut self) {
        let Some(event_loop_proxy) = self.event_loop_proxy.clone() else { return };

        if self.library_file_monitor.is_none() {
            let monitor_proxy = event_loop_proxy.clone();
            match crate::library_file_monitor::LibraryFileMonitor::spawn(move || {
                let _ =
                    monitor_proxy.send_event(crate::app_event::AppEvent::FileSafetyResultsReady);
            }) {
                Ok(monitor) => {
                    self.library_file_monitor = Some(monitor);
                    self.refresh_file_monitor_roots();
                    self.file_safety_next_check = Instant::now();
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
        self.workspace
            .entries()
            .iter()
            .filter_map(|entry| entry.value.file_path.as_deref())
            .filter_map(|path| path.parent())
            .map(Path::to_owned)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub(crate) fn refresh_file_monitor_roots(&self) {
        let Some(monitor) = self.library_file_monitor.as_ref() else { return };
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
        if matches!(self.settings.view_mode, ui::view_mode::ViewMode::Sidebar) {
            return ui::title_bar::title_bar_height(metrics.dpi) + search_h;
        }

        // Tabs mode: tab bar + search bar
        let tbh = self.current_tab_bar_height_with_metrics(&metrics);
        tbh + search_h
    }

    pub(crate) fn visible_rows(&self, screen_height: f32) -> usize {
        self.visible_height_lines(screen_height).floor() as usize
    }

    pub(crate) fn visible_height_lines(&self, screen_height: f32) -> f64 {
        let metrics = self.ui_metrics();
        let status_h = if self.settings.show_status_bar { metrics.status_bar_height } else { 0.0 };
        ((screen_height - status_h - self.content_top_offset()) / metrics.line_height).max(1.0)
            as f64
    }

    /// Compute explicit `ViewportDimensions` from instance settings and screen height.
    pub(crate) fn viewport_dimensions(
        &self,
        screen_height: f32,
    ) -> crate::workspace::ViewportDimensions {
        crate::workspace::ViewportDimensions {
            visible_rows: self.visible_rows(screen_height),
            viewport_height: self.visible_height_lines(screen_height),
        }
    }

    pub(crate) fn screen_width(&self) -> f32 {
        self.gpu.as_ref().map(|g| g.ctx.config.width as f32).unwrap_or(800.0)
    }

    pub(crate) fn screen_height(&self) -> f32 {
        self.gpu.as_ref().map(|g| g.ctx.config.height as f32).unwrap_or(600.0)
    }

    pub(crate) fn viewport_content_width(
        &self,
        document: &impl crate::edit_transaction::DocumentModelRef,
    ) -> f32 {
        let metrics = self.ui_metrics();
        let left_margin = self.toc_left_offset()
            + self
                .editor_left_margin_with_metrics(document.document_model().line_count(), &metrics);
        let editor_rect = self.ui_shell.editor_rect();
        let editor_right = editor_rect.x + editor_rect.w;
        let physical_w = editor_right - left_margin;
        const NO_WRAP_SENTINEL: f32 = 1_000_000.0;
        if self.settings.word_wrap { physical_w.max(1.0) } else { NO_WRAP_SENTINEL }
    }

    pub(crate) fn current_tab_bar_height(&self) -> f32 {
        self.current_tab_bar_height_with_metrics(&self.ui_metrics())
    }

    pub(crate) fn current_tab_bar_height_with_metrics(
        &self,
        metrics: &ui::settings::UiMetrics,
    ) -> f32 {
        if self.settings.view_mode == ui::view_mode::ViewMode::Tabs && self.workspace.len() > 1 {
            ui::tab_bar::tab_bar_height(metrics.dpi)
        } else {
            0.0
        }
    }

    /// Return UI metrics derived from current settings.
    /// Settings holds logical (pre-DPI-scale) values; this multiplies by
    /// the current scale factor to produce physical pixel values.
    pub(crate) fn ui_metrics(&self) -> ui::settings::UiMetrics {
        ui::settings::UiMetrics::from_settings(&self.settings, self.scale_factor as f32)
    }

    /// Store the new scale factor. Does NOT modify dimensional settings fields.
    pub(crate) fn update_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor =
            if scale_factor.is_finite() && scale_factor > 0.0 { scale_factor } else { 1.0 };
    }

    /// Logical (pre-DPI-scale) font size, for persistence.
    pub(crate) fn persisted_font_size(&self) -> f32 {
        self.settings.font_size
    }

    /// Set font size from a logical (pre-DPI-scale) value.
    pub(crate) fn set_logical_font_size(&mut self, logical_size: f32) {
        self.settings.set_font_size(logical_size);
    }

    /// Convert the current physical sidebar width to logical units for persistence.
    pub(crate) fn sidebar_width_for_persistence(&self) -> f32 {
        let dpi = self.scale_factor as f32;
        self.ui_shell.sidebar_width() / dpi.max(f32::EPSILON)
    }

    pub(crate) fn persist_editor_settings(&self) -> std::io::Result<()> {
        crate::settings_io::save_editor_settings(&self.paths.settings_file, &self.settings)
    }

    pub(crate) fn sync_window_chrome(&self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        match self.settings.view_mode {
            ui::view_mode::ViewMode::Sidebar => {
                crate::sys::macos_titlebar::enable_full_size_content(window);
            }
            ui::view_mode::ViewMode::Tabs => {
                crate::sys::macos_titlebar::disable_full_size_content(window);
            }
        }
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
                    self.needs_redraw = true;
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod product_ownership_boundary_tests {
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

    use super::App;

    #[test]
    fn public_open_document_sender_routes_paths_to_product_inbox() {
        let mut app = App::new(None);
        let open_document_paths = vec![PathBuf::from("/tmp/from-application-open-urls.md")];

        app.open_document_sender()
            .send(open_document_paths.clone())
            .expect("new application owns the product open-document receiver");

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
    pub(crate) fn switch_workspace_for_test(&mut self, index: usize) {
        let effect = self.workspace.switch_to(index);
        effect.reconcile_runtime_store(&mut self.tab_runtime_store);
    }
}

#[cfg(test)]
mod geometry_metrics_tests {
    use super::App;
    use crate::document_view::DocumentView;
    use crate::plugins::editor::EditorPlugin;

    #[test]
    fn app_geometry_uses_metrics_snapshot() {
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentView::new(vec!["first".into()], 80, 10.0),
            Box::new(EditorPlugin::new()),
        );
        app.push_entry_for_test(
            DocumentView::new(vec!["second".into()], 80, 10.0),
            Box::new(EditorPlugin::new()),
        );
        app.switch_workspace_for_test(0);
        app.update_scale_factor(2.0);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;

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
    use crate::document_view::DocumentView;
    use crate::plugins::editor::EditorPlugin;

    use std::path::PathBuf;

    #[test]
    fn file_monitor_roots_cover_all_open_file_parents_once() {
        let mut app = App::new(None);
        let mut first = DocumentView::new(vec!["first".into()], 10, 10.0);
        first.file_path = Some(PathBuf::from("/library/notes/first.md"));
        let mut second = DocumentView::new(vec!["second".into()], 10, 10.0);
        second.file_path = Some(PathBuf::from("/library/notes/second.md"));
        let mut third = DocumentView::new(vec!["third".into()], 10, 10.0);
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
#[path = "settings_boundary_tests.rs"]
mod settings_boundary_tests;

#[cfg(test)]
mod edit_reset_tests {
    use super::reset_cursor_after_edit;
    use crate::cursor_motion::CursorRenderState;

    #[test]
    fn cursor_reset_does_not_own_reshape_generation() {
        let mut state = CursorRenderState::new();
        state.sticky_x_dirty = false;
        let before = state.cursor_blink_instant;

        reset_cursor_after_edit(&mut state);

        assert!(state.sticky_x_dirty);
        assert!(state.cursor_blink_instant >= before);
    }
}
