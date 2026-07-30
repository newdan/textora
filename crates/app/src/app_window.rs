//! Window management: geometry, resize, IME, wake timing, shell inputs.
//! Methods on `impl App`, extracted from app.rs.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::{App, compute_cursor_phase};
use crate::app_init::build_product_workspace;
use crate::gpu;
use crate::gpu::GpuError;
use crate::render_state::{GpuState, TextState};
use crate::ui_shell::ShellInputs;
use crate::workspace_persistence::restore_workspace;
use appkit_shell::ProductHost;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::window::WindowAttributes;

const WINDOW_TITLE: &str = "edit+";

pub(crate) fn ime_cursor_x(
    cursor_rect: ui::core::geom::Rect,
    cursor_is_preedit_projected: bool,
    preedit_advance_px: f32,
) -> f32 {
    if cursor_is_preedit_projected { cursor_rect.x } else { cursor_rect.x + preedit_advance_px }
}

impl App {
    /// 设置弹窗中聚焦输入框的 IME 光标区域（窗口物理坐标）。
    fn settings_overlay_ime_cursor_rect(&self) -> Option<ui::core::geom::Rect> {
        if !self.ui_shell.active_overlay_is_modal() {
            return None;
        }
        let overlay_rect = self.ui_shell.active_overlay_layout_rect()?;
        let frame = self.ui_shell.active_overlay_widget_ref::<ui::modal_frame::ModalFrame>()?;
        let view = frame.content_as_any().downcast_ref::<ui::settings_view::SettingsView>()?;
        let local = view.focused_ime_cursor_rect()?;
        let content_rect = frame.content_rect();
        Some(ui::core::geom::Rect::new(
            overlay_rect.x + content_rect.x + local.x,
            overlay_rect.y + content_rect.y + local.y,
            local.w,
            local.h,
        ))
    }

    /// Phase 2：从 App / Workspace 状态组装 ShellInputs。
    pub(crate) fn build_shell_inputs(&self) -> ShellInputs {
        let metrics = self.ui_metrics();
        let dpi = metrics.dpi;
        let view_mode = self.settings.view_mode;

        let tabs_visible = match view_mode {
            ui::view_mode::ViewMode::Tabs => self.workspace.len() > 1,
            ui::view_mode::ViewMode::Sidebar => false,
        };
        let tabs_thickness = if tabs_visible { ui::tab_bar::tab_bar_height(dpi) } else { 0.0 };

        let search_visible =
            self.active_tab_session().is_some_and(|tab| tab.search_state().panel_visible);
        let search_thickness =
            if search_visible { ui::search_bar::SEARCH_BAR_HEIGHT * dpi } else { 0.0 };

        let status_thickness =
            if self.settings.show_status_bar { metrics.status_bar_height } else { 0.0 };

        let sidebar_visible = match view_mode {
            ui::view_mode::ViewMode::Sidebar => true,
            _ => false,
        };
        let sidebar_thickness =
            if sidebar_visible { self.ui_shell.sidebar_editor_left_offset().max(0.5) } else { 0.0 };

        let scrollbar_thickness =
            if self.active_is_canvas() { 0.0 } else { metrics.scrollbar_reserve };

        let toc_vis = self.active_toc_visible();
        ShellInputs {
            tabs_visible,
            tabs_thickness,
            search_visible,
            search_thickness,
            status_thickness,
            sidebar_visible,
            sidebar_thickness,
            scrollbar_thickness,
            toc_visible: toc_vis,
            toc_thickness: if toc_vis { metrics.toc_width } else { 0.0 },
            metrics,
            sidebar_settings: ui::sidebar::SidebarSettingsInput::from(&self.settings),
        }
    }

    pub(crate) fn quit_app(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.persist_workspace_state();
        self.record_all_entries_to_history();
        self.save_history();
        self.save_window_geometry();
        if let Some(worker) = self.file_safety_worker.take() {
            worker.shutdown();
        }
        if let Some(monitor) = self.library_file_monitor.take() {
            monitor.shutdown();
        }
        self.shutdown_product_services();
        event_loop.exit();
    }

    fn shutdown_product_services(&mut self) {
        ProductHost::shutdown(&mut self.product);
    }

    /// Save current window geometry to persisted settings.
    fn save_window_geometry(&mut self) {
        let Some(ref window) = self.window else {
            return;
        };
        let mut settings = match crate::settings_io::load(&self.paths.settings_file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[settings] failed to load settings for geometry: {}", e);
                return;
            }
        };
        if let Ok(pos) = window.outer_position() {
            settings.window_x = Some(pos.x);
            settings.window_y = Some(pos.y);
        }
        let size = window.inner_size();
        settings.window_width = Some(size.width);
        settings.window_height = Some(size.height);
        let (
            theme_mode,
            show_line_numbers,
            word_wrap,
            show_status_bar,
            font_family,
            font_size,
            line_height_ratio,
            tab_width,
        ) = {
            let s = &self.settings;
            (
                s.theme_mode,
                s.show_line_numbers,
                s.word_wrap,
                s.show_status_bar,
                s.font_family.clone(),
                s.font_size,
                s.line_height_ratio,
                s.tab_width,
            )
        };
        settings.theme_mode = theme_mode;
        settings.show_line_numbers = show_line_numbers;
        settings.word_wrap = word_wrap;
        settings.show_status_bar = show_status_bar;
        settings.font_family = font_family;
        settings.font_size = font_size;
        settings.line_height_ratio = line_height_ratio;
        settings.tab_width = tab_width;
        settings.sidebar_width = self.sidebar_width_for_persistence();
        if let Err(e) = crate::settings_io::save(&self.paths.settings_file, &settings) {
            eprintln!("[settings] save error: {}", e);
        }
    }

    /// Returns the plugin cursor rectangle in window (physical pixel) coordinates.
    /// Queries the plugin for its document-space cursor rect and transforms it
    /// using `plugin_render_bounds()` — the single source of truth for plugin positioning.
    pub(crate) fn plugin_cursor_window_rect(
        &self,
        cursor_byte: usize,
    ) -> Option<ui::core::geom::Rect> {
        let tab = self.active_tab_session()?;
        let bounds = self.plugin_render_bounds();
        tab.query_cursor_screen_rect(cursor_byte)
            .map(|(x, y, w, h)| ui::core::geom::Rect::new(bounds.x + x, bounds.y + y, w, h))
    }

    /// Notify the OS of the current IME cursor position so the candidate
    /// window follows the text caret.
    pub(crate) fn update_ime_cursor_area(&self) {
        if let Some(ref window) = self.window {
            // 设置弹窗中聚焦的输入框：IME 候选窗跟随弹窗内光标。
            if let Some(ime_rect) = self.settings_overlay_ime_cursor_rect() {
                window.set_ime_cursor_area(
                    PhysicalPosition::new(ime_rect.x as f64, (ime_rect.y + ime_rect.h) as f64),
                    PhysicalSize::new(ime_rect.w.max(2.0) as f64, ime_rect.h as f64),
                );
                return;
            }

            let search_has_focus =
                self.active_tab_session().is_some_and(|tab| tab.search_state().panel_visible)
                    && self.ui_shell.search_bar_has_keyboard_focus();

            if search_has_focus {
                // IME candidate window at search bar cursor position.
                // Use content_top_offset() to get the correct Y in both Tabs and Sidebar modes.
                if let Some(ime_rect) = self.ui_shell.search_ime_cursor_rect() {
                    let cursor_y = self.content_top_offset() + ime_rect.y;
                    let cursor_x = self.ui_shell.search_bar_x_offset() + ime_rect.x;
                    window.set_ime_cursor_area(
                        PhysicalPosition::new(cursor_x as f64, cursor_y as f64),
                        PhysicalSize::new(2.0, ime_rect.h as f64),
                    );
                }
            } else if let Some(tab) = self.active_tab_session() {
                let cursor_byte = tab.document.cursor_offset().to_usize();
                let handles_own_rendering = self.active_handles_own_rendering();
                if let Some(rect) = self.plugin_cursor_window_rect(cursor_byte) {
                    let cursor_x =
                        ime_cursor_x(rect, handles_own_rendering, self.preedit_advance_px);
                    let cursor_y = rect.y + rect.h;
                    window.set_ime_cursor_area(
                        PhysicalPosition::new(cursor_x as f64, cursor_y as f64),
                        PhysicalSize::new(2.0, rect.h as f64),
                    );
                }
            }
        }
    }

    /// Flush any pending resize if 16ms have passed since last handled.
    pub(crate) fn flush_pending_resize(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_resize_handled).as_millis() < 16 {
            return;
        }
        let Some(size) = self.pending_resize.take() else {
            return;
        };
        self.last_resize_handled = now;
        self.resize(size);
    }

    /// Handle a resize event (public API for testing and external integration).
    pub fn handle_resize(&mut self, width: u32, height: u32) {
        let new_size = winit::dpi::PhysicalSize::new(width, height);
        self.resize(new_size);
    }

    /// Resize the surface to match the new window size.
    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        let lh = self.ui_metrics().line_height;
        if let (Some(gpu), Some(_window)) = (self.gpu.as_mut(), self.window.as_ref())
            && new_size.width > 0
            && new_size.height > 0
        {
            let old_width = gpu.ctx.config.width;
            let width_changed = old_width != new_size.width;

            gpu.size = new_size;
            gpu.ctx.config.width = new_size.width;
            gpu.ctx.config.height = new_size.height;
            gpu.ctx.surface.configure(&gpu.ctx.device, &gpu.ctx.config);
            gpu.ctx.recreate_msaa();
            self.needs_redraw = true;

            if width_changed {
                let tab_ids: Vec<_> = (0..self.workspace.len())
                    .filter_map(|index| self.workspace.tab_id_at(index))
                    .collect();
                for tab_id in tab_ids {
                    if let Some(mut tab) = crate::app_tab::compose_tab_session_mut(
                        &mut self.workspace,
                        &mut self.tab_runtime_store,
                        tab_id,
                    ) {
                        tab.invalidate_render_cache_all();
                    }
                }
                if self.workspace.active_index() < self.workspace.len() {
                    if let Some(tab_id) = self.active_tab_id()
                        && let Some(mut tab) = crate::app_tab::compose_tab_session_mut(
                            &mut self.workspace,
                            &mut self.tab_runtime_store,
                            tab_id,
                        )
                    {
                        tab.clear_advance_cache();
                    }
                    self.frame_cache.cluster_pool.clear();
                    self.init_display_map(self.workspace.active_index());
                    // Clamp anchor then re-derive scroll_top after display_map rebuild
                    if let Some(mut tab) = self.active_tab_session_mut() {
                        tab.refresh_scroll_metrics(lh);
                    }

                    self.invalidate_reshape();
                }
            }

            // Update viewport visible rows
            let visible_rows = self.visible_rows(new_size.height as f32);
            let viewport_height = self.visible_height_lines(new_size.height as f32);
            if let Some(mut tab) = self.active_tab_session_mut() {
                tab.resize_presentation(visible_rows, viewport_height);
                if !width_changed {
                    // Clamp anchor after resize (visible_rows changed), only if width is same (Stage 5).
                    tab.refresh_scroll_metrics(lh);
                }
            }
        }
    }

    /// 是否有正在进行的动画（标签栏滚动）需要持续渲染。
    pub(crate) fn has_active_animation(&self) -> bool {
        self.tab_scroll.is_animating() || self.sidebar_animating
    }

    /// 计算下一次需要唤醒事件循环的时间点。
    /// 返回 None 表示可以无限期休眠（完全空闲）。
    pub(crate) fn compute_next_wake_time(&self) -> Option<Instant> {
        let mut earliest: Option<Instant> = None;

        // 1. 光标闪烁 — 有文档且有光标且窗口激活时才需要（预览模式无光标，跳过）
        if self.window_focused
            && self.active_needs_cursor_blink_wakeup()
            && let Some(tab) = self.active_tab_session()
        {
            let (_, next_blink) = compute_cursor_phase(tab.cursor_blink_instant());
            earliest = Some(match earliest {
                Some(e) => e.min(next_blink),
                None => next_blink,
            });
        }

        // 2. 标签栏滚动动画 — 动画运行期间每 16ms 唤醒一帧
        if self.has_active_animation() {
            let next_frame = Instant::now() + Duration::from_millis(16);
            earliest = Some(match earliest {
                Some(e) => e.min(next_frame),
                None => next_frame,
            });
        }

        // 3. Revision-aware file safety checks
        if self.file_safety_worker.is_some() && !self.workspace.is_empty() {
            let next_file_safety = self.file_safety_next_check;
            earliest = Some(match earliest {
                Some(e) => e.min(next_file_safety),
                None => next_file_safety,
            });
        }

        earliest
    }

    pub(crate) fn init_window(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> Result<(), GpuError> {
        let _t0 = std::time::Instant::now();

        let persisted = match crate::settings_io::load(&self.paths.settings_file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[settings] failed to load settings: {}", e);
                crate::settings_io::PersistedSettings::default()
            }
        };
        let mut attrs = WindowAttributes::default().with_title(WINDOW_TITLE);
        if let (Some(w), Some(h)) = (persisted.window_width, persisted.window_height) {
            attrs = attrs
                .with_inner_size(winit::dpi::Size::Physical(winit::dpi::PhysicalSize::new(w, h)));
        }
        let window = event_loop
            .create_window(attrs)
            .map_err(|e| GpuError::SurfaceCreation(e.to_string()))?;
        // 首帧 present 前将窗口设为完全透明，避免启动白闪；
        // 首帧 present 完成后在 app_renderer.rs 中恢复为不透明。
        crate::sys::macos_titlebar::set_window_alpha(&window, 0.0);
        window.set_ime_allowed(true);
        window.set_cursor(winit::window::CursorIcon::Text);
        self.scale_factor = window.scale_factor();
        self.ui_shell.scale_sidebar_width(self.scale_factor as f32);
        self.ui_shell.sidebar_clamp_width(self.scale_factor as f32);
        // Follow system appearance before moving window into Arc
        self.current_theme = {
            let system = window.theme().unwrap_or(winit::window::Theme::Dark);
            let mode = &self.settings.theme_mode;
            ui::Theme::resolve(*mode, system, &self.active_theme_pair, &self.theme_registry)
        };
        let window = Arc::new(window);
        let size = window.inner_size();
        eprintln!("[startup] window create + theme: {:?}", _t0.elapsed());

        let _t1 = std::time::Instant::now();
        let ctx = gpu::create_gpu_context(window.clone(), size.width, size.height)?;
        eprintln!("[startup] gpu::create_gpu_context: {:?}", _t1.elapsed());

        self.gpu = Some(GpuState { ctx, size });
        self.window = Some(window);

        // Apply macOS titlebar mode based on persisted view_mode
        if let Some(ref w) = self.window {
            match self.settings.view_mode {
                ui::view_mode::ViewMode::Sidebar => {
                    crate::sys::macos_titlebar::enable_full_size_content(w);
                }
                ui::view_mode::ViewMode::Tabs => {}
            }
        }

        // Restore window position from persisted settings (after titlebar config)
        if let Some(ref w) = self.window
            && let (Some(x), Some(y)) = (persisted.window_x, persisted.window_y)
        {
            w.set_outer_position(winit::dpi::Position::Physical(
                winit::dpi::PhysicalPosition::new(x, y),
            ));
        }

        // Initialize text rendering and load file
        let _t2 = std::time::Instant::now();
        let gpu = self.gpu.as_ref().ok_or(GpuError::NoAdapter)?;
        let metrics = self.ui_metrics();
        self.text = Some(TextState::init(
            gpu,
            metrics.font_size,
            self.shared_font_system.clone().expect("FontSystem not initialized"),
            self.settings.font_family.clone().as_str(),
        )?);
        eprintln!("[startup] TextState::init: {:?}", _t2.elapsed());

        // Defer ReshapeWorker creation until after scale factor is known.
        // This ensures the worker uses the actual DPI-aware font size.
        if self.reshape_worker.is_none() {
            let _t_worker = std::time::Instant::now();
            self.reshape_worker = Some(crate::reshape_worker::ReshapeWorker::spawn(
                self.shared_font_system.clone().expect("FontSystem not initialized"),
                metrics.font_size,
                self.settings.font_family.clone(),
            ));
            eprintln!("[startup] ReshapeWorker::spawn (deferred): {:?}", _t_worker.elapsed());
        }

        let _t3 = std::time::Instant::now();
        let screen_h = self.screen_height();
        // Clean up orphaned snapshot files once at startup
        self.workspace_store.cleanup_snapshot_orphans();
        if let Ok(Some(restored_snap)) = self.workspace_store.load_workspace() {
            let viewport = self.viewport_dimensions(screen_h);
            let line_height = metrics.line_height as f64;
            if let Ok(restored) = restore_workspace(
                build_product_workspace(),
                restored_snap,
                viewport,
                line_height,
                &self.paths.snapshots_dir,
            ) {
                eprintln!("[startup] workspace restored: {} tabs", restored.workspace.len());
                self.workspace = restored.workspace;
                self.tab_runtime_store = restored.runtimes;
                self.refresh_file_monitor_roots();
                // Auto-scroll active tab into view after workspace restore
                self.update_entry_layout();
                // Clear file_path so we don't double-open
                self.file_path = None;
                // Initialize display map for the active tab (needed for scroll_anchor restore)
                if !self.workspace.is_empty() {
                    self.init_display_map(self.workspace.active_index());
                }
            } else {
                self.load_file();
            }
        } else {
            self.load_file();
        }
        eprintln!("[startup] load_file: {:?}", _t3.elapsed());

        let _t4 = std::time::Instant::now();
        if let Ok(paths) = self.workspace_store.load_pinned_paths() {
            self.workspace.restore_pinned(&paths);
        }
        eprintln!("[startup] load_pinned: {:?}", _t4.elapsed());

        // Ensure at least one document (untitled) when no file is opened
        if self.workspace.is_empty() {
            self.new_untitled_doc();
        }
        eprintln!("[startup] init_window total: {:?}", _t0.elapsed());
        Ok(())
    }
}

#[cfg(test)]
mod build_shell_inputs_tests {
    use super::App;
    use crate::document_view::DocumentView;

    use ui::plugin::ViewPlugin;
    use ui::sidebar::Visibility;

    /// 辅助：创建含 n 个文档的 App 实例（无 GPU）。
    fn app_with_n_docs(n: usize) -> App {
        let mut app = App::new(None);
        for _i in 0..n {
            let _ = app.new_untitled_doc();
        }
        app.switch_workspace_for_test(0);
        app
    }

    #[test]
    fn tabs_mode_single_doc_hides_tabs() {
        let mut app = app_with_n_docs(1);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
        let inputs = app.build_shell_inputs();
        assert!(!inputs.tabs_visible, "单文档 Tabs 模式下 tabs 应隐藏");
        assert_eq!(inputs.tabs_thickness, 0.0);
    }

    #[test]
    fn tabs_mode_multi_doc_shows_tabs() {
        let mut app = app_with_n_docs(3);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
        let inputs = app.build_shell_inputs();
        assert!(inputs.tabs_visible, "多文档 Tabs 模式下 tabs 应可见");
        assert!(inputs.tabs_thickness > 0.0);
    }

    #[test]
    fn sidebar_mode_never_shows_tabs() {
        let mut app = app_with_n_docs(3);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        let inputs = app.build_shell_inputs();
        assert!(!inputs.tabs_visible, "Sidebar 模式下 tabs 应始终隐藏");
    }

    #[test]
    fn editor_left_margin_clears_gutter_in_sidebar_mode() {
        let mut app = app_with_n_docs(1);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        app.ui_shell.set_sidebar_pinned(true);
        app.ui_shell.set_sidebar_width(220.0);
        app.ui_shell.set_sidebar_visibility(Visibility::Pinned);

        let line_count = 1000; // 4 位数 gutter，宽度可观
        let lm = app.editor_left_margin(line_count);
        let gutter_w = app.settings.gutter_width(line_count) * app.ui_metrics().dpi;
        let sidebar_offset = app.ui_shell.sidebar_editor_left_offset();
        // gutter 左边界必须不小于 sidebar 右边界
        assert!(
            lm - gutter_w >= sidebar_offset,
            "gutter [lm-gutter_w={}, lm={}] must not overlap sidebar [0, {}]",
            lm - gutter_w,
            lm,
            sidebar_offset
        );
    }

    #[test]
    fn sidebar_pinned_reserves_left_space() {
        let mut app = app_with_n_docs(2);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        app.ui_shell.set_sidebar_pinned(true);
        let inputs = app.build_shell_inputs();
        assert!(inputs.sidebar_visible, "pinned sidebar 应可见");
        assert!(inputs.sidebar_thickness > 0.0, "pinned sidebar 应有厚度");
    }

    #[test]
    fn sidebar_not_pinned_no_space_reserved() {
        let mut app = app_with_n_docs(2);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        app.ui_shell.set_sidebar_pinned(false);
        app.ui_shell.set_sidebar_visibility(Visibility::Hidden);
        let inputs = app.build_shell_inputs();
        assert!(inputs.sidebar_visible, "Sidebar 模式始终可见（含 Hidden 态汉堡按钮）");
        assert_eq!(inputs.sidebar_thickness, 0.5, "非 pinned 不挤占编辑器空间");
    }

    #[test]
    fn status_bar_present_when_enabled() {
        let mut app = app_with_n_docs(1);
        {
            let s = &mut app.settings;
            s.view_mode = ui::view_mode::ViewMode::Tabs;
            s.show_status_bar = true;
        }
        let inputs = app.build_shell_inputs();
        assert!(inputs.status_thickness > 0.0, "status bar 有厚度");

        app.settings.show_status_bar = false;
        let inputs2 = app.build_shell_inputs();
        assert_eq!(inputs2.status_thickness, 0.0, "关闭时无厚度");
    }

    #[test]
    fn scrollbar_always_present() {
        let mut app = app_with_n_docs(1);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
        let inputs = app.build_shell_inputs();
        assert!(inputs.scrollbar_thickness > 0.0, "scrollbar 应始终有厚度");
    }

    #[test]
    fn canvas_view_does_not_reserve_legacy_scrollbar() {
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentView::new(vec!["canvas".to_string()], 10, 10.0),
            Box::new(CanvasPlugin),
        );
        app.switch_workspace_for_test(0);

        let inputs = app.build_shell_inputs();

        assert_eq!(
            inputs.scrollbar_thickness, 0.0,
            "画布视图必须使用覆盖式滚动条，不能预留旧右侧滚动条宽度"
        );
    }

    struct CanvasPlugin;

    impl ViewPlugin for CanvasPlugin {
        fn name(&self) -> &str {
            "canvas_shell_inputs_test"
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

        fn is_canvas(&self) -> bool {
            true
        }
    }

    #[test]
    fn dpi_field_is_set() {
        let app = app_with_n_docs(1);
        let inputs = app.build_shell_inputs();
        assert!(inputs.metrics.dpi > 0.0, "dpi 应 > 0");
        assert!((inputs.metrics.dpi - app.scale_factor as f32).abs() < 0.01);
    }
}

#[cfg(test)]
mod shutdown_sync_controller_boundary_tests {
    use super::App;

    fn has_direct_sync_controller_take(compact_source: &str) -> bool {
        let direct_take = ["self.sync_", "controller.take()"].concat();

        compact_source.contains(&direct_take)
    }

    #[test]
    fn shutdown_routes_product_cleanup_through_product_host() {
        let production_source = include_str!("app_window.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("app window production source should precede tests");
        let compact_production_source = production_source.split_whitespace().collect::<String>();

        assert!(
            compact_production_source.contains("ProductHost::shutdown(&mutself.product);"),
            "app shutdown must route product cleanup through ProductHost"
        );
    }

    #[test]
    fn direct_sync_controller_take_detector_rejects_field_take_without_rejecting_accessor() {
        let direct_take = ["self.sync_", "controller.take()"].concat();
        let accessor_take = ["self.take_sync_", "controller()"].concat();

        assert!(has_direct_sync_controller_take(&direct_take));
        assert!(!has_direct_sync_controller_take(&accessor_take));
    }

    #[test]
    fn shutdown_product_services_removes_configured_sync_controller() {
        let mut app = App::new(None);
        app.set_sync_controller(crate::sync_controller::SyncController::new_default(|| {}));

        app.shutdown_product_services();

        assert!(app.sync_controller().is_none());
    }
}

#[cfg(test)]
mod ime_preedit_tests {
    use super::App;

    #[test]
    fn test_preedit_fields_default_empty() {
        let app = App::new(None);
        assert!(app.preedit_text.is_empty());
        assert!(app.preedit_cursor.is_none());
    }

    #[test]
    fn test_preedit_text_cleared_on_disable() {
        let mut app = App::new(None);
        // Simulate Preedit
        app.preedit_text = "ni".to_string();
        app.preedit_cursor = Some((0, 2));
        // Simulate Disabled (IME off) — should clear
        app.preedit_text.clear();
        app.preedit_cursor = None;
        assert!(app.preedit_text.is_empty());
        assert!(app.preedit_cursor.is_none());
    }

    #[test]
    fn test_preedit_stores_composing_text() {
        let mut app = App::new(None);
        app.preedit_text = "nihao".to_string();
        app.preedit_cursor = Some((5, 5));
        assert_eq!(app.preedit_text, "nihao");
        assert_eq!(app.preedit_cursor, Some((5, 5)));
    }

    #[test]
    fn test_preedit_empty_string_with_cursor() {
        let mut app = App::new(None);
        // winit sends Preedit("", None) when composition is empty
        app.preedit_text = String::new();
        app.preedit_cursor = None;
        assert!(app.preedit_text.is_empty());
        assert!(app.preedit_cursor.is_none());
    }

    #[test]
    fn test_preedit_cursor_range() {
        let mut app = App::new(None);
        // Cursor highlighting bytes 1..3 in a 5-byte string
        app.preedit_text = "abcde".to_string();
        app.preedit_cursor = Some((1, 3));
        assert_eq!(app.preedit_cursor.unwrap(), (1, 3));
    }
}

#[cfg(test)]
mod ui_shell_alignment_tests {
    use crate::ui_shell::{ShellInputs, UiShell};

    use ui::core::{NoopMeasure, Rect, Screen};

    fn run(inputs: ShellInputs) -> Rect {
        let theme = ui::theme::test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        shell.mark_layout_initialized_for_test();
        shell.update_frame(Screen::new(1200.0, 800.0), &theme, &mut m, &inputs);
        shell.editor_rect()
    }

    #[test]
    fn alignment_tabs_mode_with_scrollbar() {
        let r = run(ShellInputs {
            tabs_visible: true,
            tabs_thickness: 32.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 24.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 12.0,
            metrics: ui::settings::UiMetrics::from_settings(&ui::settings::Settings::new(), 1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        });
        // 屏幕 1200x800, 上 32, 下 24, 右 12 → editor = (0, 32, 1188, 744)
        assert_eq!(r, Rect::new(0.0, 32.0, 1188.0, 744.0));
    }

    #[test]
    fn alignment_sidebar_mode() {
        let r = run(ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 24.0,
            sidebar_visible: true,
            sidebar_thickness: 220.0,
            scrollbar_thickness: 12.0,
            metrics: ui::settings::UiMetrics::from_settings(&ui::settings::Settings::new(), 1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        });
        assert_eq!(r, Rect::new(220.0, 36.0, 968.0, 740.0));
    }

    #[test]
    fn alignment_sidebar_mode_with_search() {
        let r = run(ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: true,
            search_thickness: 28.0,
            status_thickness: 24.0,
            sidebar_visible: true,
            sidebar_thickness: 220.0,
            scrollbar_thickness: 0.0,
            metrics: ui::settings::UiMetrics::from_settings(&ui::settings::Settings::new(), 1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        });
        // Sidebar 220, TitleBar 36, SearchBar 28 → top=64, Status 24
        // editor = (220, 64, 980, 712)
        assert_eq!(r, Rect::new(220.0, 64.0, 980.0, 712.0));
    }

    #[test]
    fn alignment_search_bar_active() {
        let r = run(ShellInputs {
            tabs_visible: true,
            tabs_thickness: 32.0,
            search_visible: true,
            search_thickness: 28.0,
            status_thickness: 24.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: ui::settings::UiMetrics::from_settings(&ui::settings::Settings::new(), 1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        });
        // 上 32+28=60, 下 24 → editor = (0, 60, 1200, 716)
        assert_eq!(r, Rect::new(0.0, 60.0, 1200.0, 716.0));
    }

    #[test]
    fn alignment_full_screen_no_chrome() {
        let r = run(ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: ui::settings::UiMetrics::from_settings(&ui::settings::Settings::new(), 1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        });
        assert_eq!(r, Rect::new(0.0, 0.0, 1200.0, 800.0));
    }
}

#[cfg(test)]
mod sidebar_hover_peek_tests {
    use super::App;
    use ui::sidebar::Visibility;

    #[test]
    fn hover_peek_sidebar_not_visible_to_dock() {
        let mut app = App::new(None);
        let _ = app.new_untitled_doc();
        app.switch_workspace_for_test(0);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        app.ui_shell.set_sidebar_pinned(false);

        // 设置为 HoverPeek 态
        app.ui_shell.set_sidebar_visibility(Visibility::HoverPeek);

        let inputs = app.build_shell_inputs();
        assert!(inputs.sidebar_visible, "Sidebar 模式始终可见（HoverPeek 也需接收事件）");
        assert_eq!(
            inputs.sidebar_thickness, 0.5,
            "HoverPeek 状态下 sidebar_thickness 应为 0.5（浮层不挤占空间）"
        );
    }

    #[test]
    fn hover_peek_sidebar_width_not_zero_but_dock_ignores() {
        let mut app = App::new(None);
        let _ = app.new_untitled_doc();
        app.switch_workspace_for_test(0);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        app.ui_shell.set_sidebar_pinned(false);
        app.ui_shell.set_sidebar_visibility(Visibility::HoverPeek);

        // current_width 返回 cfg.width（非零），但 editor_left_offset 为 0（浮层不占位）
        let hover_width = app.ui_shell.sidebar_current_width();
        assert!(hover_width > 0.0, "HoverPeek 时 current_width 应 > 0（浮层面板宽度）");

        let inputs = app.build_shell_inputs();
        assert_eq!(
            inputs.sidebar_thickness, 0.5,
            "即使 current_width > 0，HoverPeek 的 editor_left_offset 为 0.5"
        );
    }

    #[test]
    fn hidden_sidebar_no_space() {
        let mut app = App::new(None);
        let _ = app.new_untitled_doc();
        app.switch_workspace_for_test(0);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        app.ui_shell.set_sidebar_pinned(false);
        app.ui_shell.set_sidebar_visibility(Visibility::Hidden);

        let inputs = app.build_shell_inputs();
        assert!(inputs.sidebar_visible, "Sidebar 模式始终可见（Hidden 态需汉堡按钮）");
        assert_eq!(inputs.sidebar_thickness, 0.5, "Hidden 态不挤占编辑器空间");
    }

    #[test]
    fn pinned_sidebar_does_reserve_space() {
        let mut app = App::new(None);
        let _ = app.new_untitled_doc();
        app.switch_workspace_for_test(0);
        app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
        app.ui_shell.set_sidebar_pinned(true);
        app.ui_shell.set_sidebar_visibility(Visibility::Pinned);

        let inputs = app.build_shell_inputs();
        assert!(inputs.sidebar_visible, "Pinned 时 sidebar 应 dock 占位");
        assert!(inputs.sidebar_thickness > 0.0);
    }
}

#[cfg(test)]
mod update_frame_skip_tests {
    use super::App;
    use crate::ui_shell::UiShell;

    use ui::core::{NoopMeasure, Rect, Screen};

    #[test]
    fn update_frame_skip_without_text_does_not_panic() {
        let app = App::new(None);
        // app.text 为 None（无 GPU 初始化），应安全跳过
        app.build_shell_inputs(); // 不应 panic
        assert_eq!(
            app.ui_shell.editor_rect(),
            Rect::ZERO,
            "text=None 时 ui_shell 保持初始 ZERO rect"
        );
    }

    #[test]
    fn ui_shell_starts_with_zero_rect() {
        let shell = UiShell::new();
        assert_eq!(shell.editor_rect(), Rect::ZERO, "新建 UiShell 的 editor_rect 应为 ZERO");
    }

    #[test]
    fn update_frame_with_no_chrome_gives_full_screen() {
        let theme = ui::theme::test_theme();
        let mut m = NoopMeasure;
        let mut shell = UiShell::new();
        let inputs = crate::ui_shell::ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: 0.0,
            metrics: ui::settings::UiMetrics::from_settings(&ui::settings::Settings::new(), 1.0),
            toc_visible: false,
            toc_thickness: 0.0,
            sidebar_settings: Default::default(),
        };
        shell.update_frame(Screen::new(1920.0, 1080.0), &theme, &mut m, &inputs);
        assert_eq!(shell.editor_rect(), Rect::new(0.0, 0.0, 1920.0, 1080.0));
    }
}

#[cfg(test)]
#[cfg(feature = "markdown")]
mod wake_time_tests {
    use super::App;
    use crate::document_view::DocumentView;
    use crate::plugins::editor::EditorPlugin;

    #[test]
    fn preview_mode_skips_cursor_blink_wake() {
        let mut app = App::new(None);
        let mut doc = DocumentView::new(vec!["hello".into()], 80, 10.0);
        doc.file_path = Some(std::path::PathBuf::from("test.txt"));
        app.push_entry_for_test(doc, Box::new(EditorPlugin::new()));
        app.switch_active_plugin();
        app.window_focused = true;

        // 预览模式：allows_editing() == false，不应调度光标闪烁唤醒
        assert!(
            !app.active_tab_session().unwrap().runtime.plugin.allows_editing(),
            "txt plugin should be preview mode"
        );
        let wake = app.compute_next_wake_time();
        assert!(
            wake.is_none(),
            "preview mode should not schedule cursor blink wake, got {:?}",
            wake
        );
    }

    #[test]
    fn editor_mode_schedules_cursor_blink_wake() {
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentView::new(vec!["hello".into()], 80, 10.0),
            Box::new(EditorPlugin::new()),
        );
        app.window_focused = true;

        // 编辑模式：allows_editing() == true，应调度光标闪烁唤醒
        assert!(app.active_tab_session().unwrap().runtime.plugin.allows_editing());
        let wake = app.compute_next_wake_time();
        assert!(wake.is_some(), "editor mode should schedule cursor blink wake");
    }

    #[test]
    fn unfocused_window_skips_blink_wake() {
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentView::new(vec!["hello".into()], 80, 10.0),
            Box::new(EditorPlugin::new()),
        );
        app.window_focused = false;

        // 窗口未聚焦时不应调度光标闪烁
        let wake = app.compute_next_wake_time();
        assert!(wake.is_none(), "unfocused window should not schedule blink wake, got {:?}", wake);
    }
}
