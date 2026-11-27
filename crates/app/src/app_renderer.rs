//! Vertex generation and GPU submission. Extracted from app.rs.
//! Methods here are part of `impl App`, just defined in a separate file.

use std::sync::OnceLock;
use std::time::Instant;

use crate::app::App;
use appkit_shell::editor_runtime::{
    EditorFrame, EditorPaintOptions, EditorRuntime, EmptyFrameTarget, PreeditAdvanceMeasure,
    PreparedPluginContent, RenderResources, SearchHighlightStyle, SurfaceFrameOptions,
    SurfaceFrameReport, SurfaceFrameSubmission, SurfaceSkipReason, TextCaretReveal,
    TextHighlightLayer, WindowCornerClip,
};
use ui::canvas::CanvasViewportSnapshot;
use ui::plugin::{PLUGIN_EDITOR, PLUGIN_MARKDOWN_EDITOR, PLUGIN_NOVEL_VIEW};
use ui::sidebar::SidebarHeaderLayout;

const PERF_LOG_ENV: &str = "EDIT_PLUS_PERF_LOG";
const PERF_LOG_THRESHOLD_US_ENV: &str = "EDIT_PLUS_PERF_LOG_THRESHOLD_US";
const DEFAULT_PERF_LOG_THRESHOLD_US: u128 = 1_000;
const WYSIWYG_CURSOR_LOG_ENV: &str = "EDIT_PLUS_WYSIWYG_CURSOR_LOG";
const SEARCH_FOCUSED_CARET_OPACITY: f32 = 0.4;
const MACOS_SIDEBAR_TOGGLE_LEFT_LOGICAL: f32 = 72.0;
const WINDOWS_SIDEBAR_TOGGLE_LEFT_LOGICAL: f32 = crate::events::WINDOW_RESIZE_MARGIN_LOGICAL + 4.0;

enum FrameRenderOutcome {
    Rendered(SurfaceFrameSubmission),
    Aborted,
    NeedsSurfaceReconfigure,
}

fn sidebar_header_layout(fullscreen: bool) -> SidebarHeaderLayout {
    if cfg!(target_os = "macos") {
        if fullscreen {
            SidebarHeaderLayout::Fullscreen
        } else {
            SidebarHeaderLayout::TitleBar { toggle_left_logical: MACOS_SIDEBAR_TOGGLE_LEFT_LOGICAL }
        }
    } else if cfg!(target_os = "windows") {
        SidebarHeaderLayout::TitleBar { toggle_left_logical: WINDOWS_SIDEBAR_TOGGLE_LEFT_LOGICAL }
    } else {
        SidebarHeaderLayout::Inline
    }
}

#[cfg(all(test, target_os = "macos"))]
mod sidebar_chrome_tests {
    use super::sidebar_header_layout;

    #[test]
    fn macos_reclaims_traffic_light_space_in_fullscreen() {
        for dpi in [1.0, 1.5, 2.0] {
            let windowed = sidebar_header_layout(false);
            assert_eq!(windowed.toggle_left_px(dpi), 72.0 * dpi);
            assert_eq!(windowed.reserved_titlebar_width_px(dpi), 104.0 * dpi);

            let fullscreen = sidebar_header_layout(true);
            assert_eq!(fullscreen.toggle_left_px(dpi), 12.0 * dpi);
            assert_eq!(fullscreen.reserved_titlebar_width_px(dpi), 44.0 * dpi);
        }
    }
}

fn perf_logging_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var(PERF_LOG_ENV).is_ok_and(|value| {
            !matches!(value.as_str(), "" | "0" | "false" | "FALSE" | "off" | "OFF")
        })
    })
}

fn perf_log_threshold_us() -> u128 {
    static THRESHOLD_US: OnceLock<u128> = OnceLock::new();
    *THRESHOLD_US.get_or_init(|| {
        std::env::var(PERF_LOG_THRESHOLD_US_ENV)
            .ok()
            .and_then(|value| value.parse::<u128>().ok())
            .unwrap_or(DEFAULT_PERF_LOG_THRESHOLD_US)
    })
}

fn should_log_perf(elapsed_us: u128) -> bool {
    perf_logging_enabled() && elapsed_us >= perf_log_threshold_us()
}

fn wysiwyg_cursor_logging_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var(WYSIWYG_CURSOR_LOG_ENV).is_ok_and(|value| {
            !matches!(value.as_str(), "" | "0" | "false" | "FALSE" | "off" | "OFF")
        })
    })
}

fn needs_redraw_after_render(
    redraw_requested_during_render: bool,
    scrollbar_is_dragging: bool,
) -> bool {
    redraw_requested_during_render || scrollbar_is_dragging
}

fn plugin_selection_pull_is_safe(mouse_is_down: bool, source_updated: bool) -> bool {
    !mouse_is_down && !source_updated
}

fn file_safety_notice_label(notice: &appkit_core::file_safety::FileSafetyNotice) -> &'static str {
    match notice {
        appkit_core::file_safety::FileSafetyNotice::CleanDocumentReloaded { .. } => {
            "已同步远端修改"
        }
        appkit_core::file_safety::FileSafetyNotice::ConflictCopyCreated { .. } => "已保留冲突副本",
        appkit_core::file_safety::FileSafetyNotice::DocumentDetachedAfterDeletion { .. } => {
            "文件已删除，内容已恢复"
        }
        appkit_core::file_safety::FileSafetyNotice::ConflictCopyFailed { .. } => "冲突保护失败",
        appkit_core::file_safety::FileSafetyNotice::AmbiguousRename { .. } => "检测到歧义重命名",
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::{App, needs_redraw_after_render, plugin_selection_pull_is_safe};
    use crate::document_fixture::DocumentFixture;
    use ui::canvas::CanvasPoint;
    use ui::plugin::{CanvasContentMetrics, ViewPlugin};

    #[test]
    fn first_frame_queues_background_services_after_present() {
        let source = include_str!("app_renderer.rs");
        let (_, render) = source
            .rsplit_once("    fn render_with_runtime_resources(")
            .expect("renderer should compose frames with runtime resources");
        let present_position = render
            .find(".submit_surface_frame_with_timing(")
            .expect("renderer should present through the shared surface submission");
        let startup_position = render
            .find("AppEvent::StartBackgroundServices")
            .expect("renderer should queue deferred background startup");

        assert!(startup_position > present_position);
    }

    #[test]
    fn render_preserves_redraw_requested_during_frame() {
        assert!(
            needs_redraw_after_render(true, false),
            "a redraw requested during render must schedule the follow-up frame"
        );
    }

    #[test]
    fn render_consumes_the_runtime_request_that_started_it() {
        let mut app = App::new(None);
        app.editor_runtime.request_redraw();

        let _ = app.render();

        assert!(!app.editor_runtime.take_redraw_request());
    }

    #[test]
    fn render_keeps_redrawing_while_scrollbar_is_dragging() {
        assert!(
            needs_redraw_after_render(false, true),
            "scrollbar dragging needs continuous redraw"
        );
    }

    #[test]
    fn render_clears_redraw_when_frame_is_settled() {
        assert!(!needs_redraw_after_render(false, false));
    }

    #[test]
    fn plugin_selection_pull_gate_blocks_stale_plugin_source() {
        assert!(!plugin_selection_pull_is_safe(false, true));
    }

    #[test]
    fn plugin_selection_pull_gate_allows_clean_idle_plugin_selection() {
        assert!(plugin_selection_pull_is_safe(false, false));
    }

    #[test]
    fn plugin_selection_pull_gate_blocks_mouse_drag() {
        assert!(!plugin_selection_pull_is_safe(true, false));
    }

    #[test]
    fn canvas_prepare_resolves_viewport_before_render_canvas() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentFixture::new(vec!["canvas".to_string()], 10, 10.0),
            Box::new(CanvasSequencePlugin { events: Rc::clone(&events) }),
        );
        app.switch_workspace_for_test(0);
        let mut shaper = shaping::Shaper::new().expect("测试画布插件必须能够创建文本整形器");

        let snapshot = app
            .prepare_active_canvas_frame(&mut shaper)
            .expect("画布 Prepare 应生成可渲染的视口快照");
        let dpi = app.ui_metrics().dpi;
        let theme = app.editor_runtime.theme().clone();
        let mut tab = app.active_tab_session_mut().expect("测试工作区必须保留活动画布标签");
        let _ = tab.render_canvas_plugin(&snapshot, &theme, &mut shaper, dpi);

        assert_eq!(&*events.borrow(), &["prepare", "render_canvas"]);
    }

    #[test]
    fn canvas_prepare_replaces_cached_legacy_scrollbar_bounds() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentFixture::new(vec!["canvas".to_string()], 10, 10.0),
            Box::new(CanvasSequencePlugin { events }),
        );
        app.switch_workspace_for_test(0);
        app.editor_runtime
            .edit_settings(|settings| settings.view_mode = ui::view_mode::ViewMode::Tabs);
        let metrics = app.ui_metrics();
        let legacy_inputs = crate::ui_shell::ShellInputs {
            tabs_visible: false,
            tabs_thickness: 0.0,
            search_visible: false,
            search_thickness: 0.0,
            status_thickness: 0.0,
            sidebar_visible: false,
            sidebar_thickness: 0.0,
            scrollbar_thickness: metrics.scrollbar_reserve,
            toc_visible: false,
            toc_thickness: 0.0,
            metrics,
            sidebar_settings: Default::default(),
        };
        let mut measure = ui::core::NoopMeasure;
        app.ui_shell.update_frame(
            ui::core::Screen::new(app.screen_width(), app.screen_height()),
            app.editor_runtime.theme(),
            &mut measure,
            &legacy_inputs,
        );
        app.ui_shell.update_frame(
            ui::core::Screen::new(app.screen_width(), app.screen_height()),
            app.editor_runtime.theme(),
            &mut measure,
            &legacy_inputs,
        );
        let mut shaper = shaping::Shaper::new().expect("测试画布插件必须能够创建文本整形器");

        let snapshot = app
            .prepare_active_canvas_frame(&mut shaper)
            .expect("画布 Prepare 应刷新过期的编辑区边界");

        assert_eq!(
            snapshot.viewport.w,
            app.screen_width(),
            "画布快照不得复用预留旧滚动条宽度的缓存 editor rect"
        );
    }

    #[test]
    fn canvas_prepare_uses_metrics_after_source_sync() {
        let state = Rc::new(RefCell::new(SourceSynchronizedCanvasState::default()));
        let mut app = App::new(None);
        let source = "synchronized canvas source";
        app.push_entry_for_test(
            DocumentFixture::new(vec![source.to_string()], 10, 10.0),
            Box::new(SourceSynchronizedCanvasPlugin { state: Rc::clone(&state) }),
        );
        app.switch_workspace_for_test(0);
        let mut shaper = shaping::Shaper::new().expect("测试画布插件必须能够创建文本整形器");

        let snapshot = app
            .sync_and_prepare_canvas_frame(&mut shaper)
            .expect("源文本同步后画布 Prepare 应生成快照");
        let state = state.borrow();

        assert_eq!(state.events, ["update_source", "prepare"]);
        assert_eq!(snapshot.content_bounds.w, source.len() as f32);
    }

    #[test]
    fn prepared_canvas_layouts_synced_overlay_and_registers_its_editor_bounds() {
        let state = Rc::new(RefCell::new(SourceSynchronizedCanvasState::new(200.0)));
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentFixture::new(vec!["canvas source".to_string()], 10, 10.0),
            Box::new(SourceSynchronizedCanvasPlugin { state: Rc::clone(&state) }),
        );
        app.switch_workspace_for_test(0);
        let mut shaper = shaping::Shaper::new().expect("测试画布插件必须能够创建文本整形器");

        let mut frame = app.editor_runtime.begin_frame().expect("无窗口的测试可以开始帧");
        let snapshot = app
            .prepare_editor_frame(&mut frame, &mut shaper)
            .expect("常规帧必须在布局覆盖层前准备画布");
        let editor_rect = app.ui_shell.editor_rect();
        app.paint_editor_surface(&mut frame, &mut Default::default(), Some(&snapshot))
            .expect("无 GPU 的画布帧也应记录编辑区位置");
        assert!(app.editor_runtime.editor_hit_test_allowed(
            appkit_shell::editor_runtime::EditorInputContext {
                focus: appkit_shell::editor_runtime::EditorFocus::Active,
                modal_blocked: false,
            },
            (editor_rect.x + 1.0, editor_rect.y + 1.0),
        ));
        let mut event_context =
            ui::core::widget::EventCtx::new(app.editor_runtime.theme(), app.ui_metrics().dpi);
        let action = app.ui_shell.dispatch(
            &ui::core::widget::Event::MouseDown {
                px: editor_rect.x + editor_rect.w * 0.5,
                py: editor_rect.bottom() - 1.0,
                button: ui::core::widget::MouseButton::Left,
            },
            &mut event_context,
        );
        assert_eq!(state.borrow().events, ["update_source", "prepare"]);
        assert!(snapshot.max_scroll.x > 0.0, "测试夹具必须产生横向覆盖滚动条");
        assert!(
            matches!(action, Some(ui::core::widget::WidgetAction::CanvasScrollbars(_))),
            "同步后的覆盖滚动条必须在当前 Shell 布局帧命中"
        );
    }

    #[test]
    fn shared_source_frame_keeps_gutter_clear_of_pinned_sidebar_at_retina_scale() {
        let mut app = App::new(None);
        app.editor_runtime
            .edit_settings(|settings| settings.set_view_mode(ui::view_mode::ViewMode::Sidebar));
        app.update_scale_factor(2.0);
        app.ui_shell.set_sidebar_pinned(true);
        app.ui_shell.set_sidebar_width_logical(240.0);
        app.ui_shell.set_sidebar_visibility(ui::sidebar::Visibility::Pinned);
        app.push_entry_for_test(
            DocumentFixture::new(vec!["source text".to_owned()], 10, 10.0),
            Box::new(crate::plugins::editor::EditorPlugin::new()),
        );
        app.switch_workspace_for_test(0);
        let mut shaper = shaping::Shaper::new().expect("行号栏测试需要文字整形器");
        let mut frame = app.editor_runtime.begin_frame().expect("应能创建无窗口编辑帧");
        assert!(app.prepare_editor_frame(&mut frame, &mut shaper).is_none());
        let dpi = app.ui_metrics().dpi;
        app.ui_shell.paint_chrome(app.editor_runtime.theme(), dpi, Some(&mut shaper));
        assert!(app.prepare_editor_frame(&mut frame, &mut shaper).is_none());

        let editor = app.ui_shell.editor_rect();
        assert!(editor.x > 0.0, "固定侧栏应占据编辑区左侧");
        let screen = ui::Screen::new(editor.right(), editor.bottom());
        let mut resources = Default::default();
        app.paint_editor_surface(&mut frame, &mut resources, None)
            .expect("纯文本布局可在无 GPU 时绘制行号背景");
        let mut vertices = Vec::new();
        frame.drain_into(screen, &mut resources, &mut vertices);

        assert_eq!(vertices.len(), 6);
        let right = screen.px_to_ndc(app.editor_left_margin(1), editor.y)[0];
        let left_edge = screen.px_to_ndc(editor.x, editor.y)[0];
        assert!(vertices.iter().all(|vertex| vertex.position[0] >= left_edge));
        assert!(vertices.iter().any(|vertex| (vertex.position[0] - right).abs() < f32::EPSILON));
    }

    struct CanvasSequencePlugin {
        events: Rc<RefCell<Vec<&'static str>>>,
    }

    impl ViewPlugin for CanvasSequencePlugin {
        fn name(&self) -> &str {
            "canvas_frame_sequence_test"
        }

        fn mode(&self) -> ui::plugin::PluginMode {
            ui::plugin::PluginMode::Canvas
        }

        fn render(
            &mut self,
            _doc: &dyn textora_core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            self.events.borrow_mut().push("render");
            ui::core::paint::DrawList::new()
        }
    }

    impl ui::plugin::ViewPluginCapabilities for CanvasSequencePlugin {
        fn canvas_mut(&mut self) -> Option<&mut dyn ui::plugin::CanvasView> {
            Some(self)
        }
    }

    impl ui::plugin::CanvasView for CanvasSequencePlugin {
        fn prepare_canvas(
            &mut self,
            _doc: &dyn textora_core::document::DocView,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> Option<CanvasContentMetrics> {
            self.events.borrow_mut().push("prepare");
            Some(CanvasContentMetrics {
                content_bounds: ui::core::geom::Rect::new(0.0, 0.0, 1_600.0, 1_200.0),
                focus_anchor: Some(CanvasPoint::new(0.0, 0.0)),
            })
        }

        fn render_canvas(
            &mut self,
            _doc: &dyn textora_core::document::DocView,
            _viewport: &ui::canvas::CanvasViewportSnapshot,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            self.events.borrow_mut().push("render_canvas");
            ui::core::paint::DrawList::new()
        }
    }

    struct SourceSynchronizedCanvasState {
        events: Vec<&'static str>,
        source: String,
        generation: Option<u32>,
        content_width_scale: f32,
    }

    impl SourceSynchronizedCanvasState {
        fn new(content_width_scale: f32) -> Self {
            Self {
                events: Vec::new(),
                source: String::new(),
                generation: None,
                content_width_scale,
            }
        }
    }

    impl Default for SourceSynchronizedCanvasState {
        fn default() -> Self {
            Self::new(1.0)
        }
    }

    struct SourceSynchronizedCanvasPlugin {
        state: Rc<RefCell<SourceSynchronizedCanvasState>>,
    }

    impl ViewPlugin for SourceSynchronizedCanvasPlugin {
        fn name(&self) -> &str {
            "source_synchronized_canvas_test"
        }

        fn mode(&self) -> ui::plugin::PluginMode {
            ui::plugin::PluginMode::Canvas
        }

        fn render(
            &mut self,
            _doc: &dyn textora_core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            ui::core::paint::DrawList::new()
        }

        fn sync_document(
            &mut self,
            generation: u32,
            reason: ui::plugin::SyncReason,
            load_source: &mut dyn FnMut() -> String,
        ) -> ui::plugin::SyncOutcome {
            if reason == ui::plugin::SyncReason::Normal
                && self.state.borrow().generation == Some(generation)
            {
                return ui::plugin::SyncOutcome::Unchanged;
            }
            let mut state = self.state.borrow_mut();
            state.events.push("update_source");
            state.source = load_source();
            state.generation = Some(generation);
            ui::plugin::SyncOutcome::Updated
        }
    }

    impl ui::plugin::ViewPluginCapabilities for SourceSynchronizedCanvasPlugin {
        fn canvas_mut(&mut self) -> Option<&mut dyn ui::plugin::CanvasView> {
            Some(self)
        }
    }

    impl ui::plugin::CanvasView for SourceSynchronizedCanvasPlugin {
        fn prepare_canvas(
            &mut self,
            _doc: &dyn textora_core::document::DocView,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> Option<CanvasContentMetrics> {
            let mut state = self.state.borrow_mut();
            state.events.push("prepare");
            Some(CanvasContentMetrics {
                content_bounds: ui::core::geom::Rect::new(
                    0.0,
                    0.0,
                    state.source.len() as f32 * state.content_width_scale,
                    100.0,
                ),
                focus_anchor: None,
            })
        }

        fn render_canvas(
            &mut self,
            _doc: &dyn textora_core::document::DocView,
            _viewport: &ui::canvas::CanvasViewportSnapshot,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            self.state.borrow_mut().events.push("render_canvas");
            ui::core::paint::DrawList::new()
        }
    }
}

impl App {
    fn file_safety_status_label(&self) -> Option<String> {
        self.workspace_services
            .file_safety_notices()
            .last()
            .map(file_safety_notice_label)
            .map(str::to_owned)
    }

    /// Compute the left margin for editor content positioning.
    /// Accounts for gutter width, content margin, and sidebar offset.
    ///
    /// 注意：editor_left_margin 是**文字内容**起点，gutter 在它**左侧** 绘制
    /// （[lm - gutter_w, lm]）。在 sidebar pinned 模式下必须保证
    /// `lm - gutter_w >= sidebar_offset + gap`，否则 gutter 会被 sidebar 覆盖。
    pub(crate) fn editor_left_margin(&self, line_count: usize) -> f32 {
        self.editor_left_margin_with_metrics(line_count, &self.ui_metrics())
    }

    pub(crate) fn editor_left_margin_with_metrics(
        &self,
        line_count: usize,
        metrics: &ui::settings::UiMetrics,
    ) -> f32 {
        let gutter_w = self.editor_runtime.settings().gutter_width(line_count) * metrics.dpi;
        let lm = metrics.content_left_margin.max(gutter_w);

        if matches!(self.editor_runtime.settings().view_mode, ui::view_mode::ViewMode::Sidebar) {
            let sidebar_offset = self.ui_shell.sidebar_editor_left_offset(metrics.dpi);
            if sidebar_offset > 0.0 {
                let gap = 10.0 * metrics.dpi;
                lm.max(sidebar_offset + gap + gutter_w)
            } else {
                lm
            }
        } else {
            lm
        }
    }

    /// Left offset for the TOC panel (only when TOC is visible in active markdown view).
    pub(crate) fn toc_left_offset(&self) -> f32 {
        let metrics = self.ui_metrics();
        let toc_vis = self.active_toc_visible();
        // Only the gap between TOC panel and content; the dock already
        // allocates toc_width via the Side::Left TocWidget child.
        if toc_vis { 10.0 * metrics.dpi } else { 0.0 }
    }

    pub(crate) fn render(&mut self) -> Option<SurfaceFrameSubmission> {
        // This frame satisfies requests made before painting. The frame tail
        // handles fresh requests raised during preparation or painting.
        self.editor_runtime.take_redraw_request();
        let mut frame = self.editor_runtime.begin_frame().ok()?;
        let outcome = EditorRuntime::with_render_resources_for(
            self,
            |app| &mut app.editor_runtime,
            |app, resources| app.render_with_runtime_resources(resources, &mut frame),
        );
        match outcome {
            FrameRenderOutcome::Rendered(submission) => Some(submission),
            FrameRenderOutcome::Aborted => None,
            FrameRenderOutcome::NeedsSurfaceReconfigure => {
                if let Some((width, height)) = self.editor_runtime.surface_size() {
                    let _ = self.editor_runtime.resize_now(width, height);
                }
                self.platform.schedule_redraw();
                None
            }
        }
    }

    fn render_with_runtime_resources(
        &mut self,
        resources: &mut RenderResources,
        frame: &mut EditorFrame,
    ) -> FrameRenderOutcome {
        let _render_t0 = std::time::Instant::now();
        let is_first_frame = !self.editor_runtime.first_frame_presented();
        let _frame_interval_us = self.editor_runtime.note_render_started(_render_t0);
        if self.editor_is_empty() || self.active_editor_index().is_none() {
            if is_first_frame {
                self.platform
                    .log_startup_point("first_frame_aborted", format_args!(" reason=empty_editor"));
            }
            return FrameRenderOutcome::Aborted;
        }
        if let Some(text) = resources.text.as_mut() {
            text.begin_frame();
        }
        self.platform.clear_redraw();
        let screen_w = self.screen_width();
        let screen_h = self.screen_height();

        let sync_prepare_started_at = Instant::now();
        let canvas = if let Some(text) = resources.text.as_mut() {
            self.prepare_editor_frame(frame, &mut text.shaper)
        } else {
            self.sync_plugin_state();
            self.ui_shell.set_canvas_scrollbars_input(None);
            None
        };
        let perf_sync_prepare_us = sync_prepare_started_at.elapsed().as_micros();

        let editor_paint_started_at = Instant::now();
        let Some(()) = self.paint_editor_surface(frame, resources, canvas.as_ref()) else {
            if is_first_frame {
                self.platform.log_startup_point(
                    "first_frame_aborted",
                    format_args!(" reason=paint_editor_none"),
                );
            }
            return FrameRenderOutcome::Aborted;
        };
        let perf_editor_paint_us = editor_paint_started_at.elapsed().as_micros();

        let paint_chrome_started_at = Instant::now();
        frame.with_paint_context(|context| {
            let chrome_list = self.ui_shell.paint_chrome(
                context.theme,
                context.dpi,
                context.shaper.as_deref_mut(),
            );
            context.list.cmds.extend(chrome_list.cmds);
        });
        let perf_paint_chrome_us = paint_chrome_started_at.elapsed().as_micros();

        let mut vertices = Vec::new();
        let frame_drain_started_at = Instant::now();
        frame.drain_into(ui::core::Screen::new(screen_w, screen_h), resources, &mut vertices);
        let perf_chrome_drain_us = frame_drain_started_at.elapsed().as_micros();

        let [red, green, blue, _] = self.editor_runtime.theme().editor.background;
        let submit_result = resources.submit_surface_frame_with_timing(
            &vertices,
            SurfaceFrameOptions {
                background: [red, green, blue, 1.0],
                empty_frame_target: EmptyFrameTarget::MultisampledWhenTextAvailable,
                window_corner_clip: WindowCornerClip::Preserve,
            },
            self.platform.has_pending_input_sample(),
        );
        let submission = match submit_result {
            Ok(Some(submission)) => submission,
            Ok(None) => {
                let report = resources.last_surface_report();
                if is_first_frame {
                    self.platform.log_startup_point(
                        "first_frame_submit_skipped",
                        format_args!(" report={report:?} err=None"),
                    );
                }
                return match report {
                    Some(SurfaceFrameReport::Skipped(SurfaceSkipReason::Occluded)) => {
                        self.platform.defer_redraw_until_unoccluded();
                        FrameRenderOutcome::Aborted
                    }
                    Some(SurfaceFrameReport::Skipped(SurfaceSkipReason::Timeout)) => {
                        self.platform.schedule_redraw();
                        FrameRenderOutcome::Aborted
                    }
                    Some(SurfaceFrameReport::Skipped(
                        SurfaceSkipReason::Outdated | SurfaceSkipReason::Lost,
                    )) => FrameRenderOutcome::NeedsSurfaceReconfigure,
                    _ => FrameRenderOutcome::Aborted,
                };
            }
            Err(error) => {
                let report = resources.last_surface_report();
                if is_first_frame {
                    self.platform.log_startup_point(
                        "first_frame_submit_skipped",
                        format_args!(" report={report:?} err={error:?}"),
                    );
                }
                return FrameRenderOutcome::Aborted;
            }
        };
        let perf_gpu_submit_us = submission.submit_and_present.as_micros();

        if !self.editor_runtime.first_frame_presented() {
            self.editor_runtime.mark_frame_presented();
            self.platform.log_startup_stage(
                "first_frame_visible",
                _render_t0.elapsed(),
                format_args!(
                    " sync_prepare={:.2}ms editor_paint={:.2}ms paint_chrome={:.2}ms drain={:.2}ms surface={:?} vertices={}",
                    perf_sync_prepare_us as f64 / 1_000.0,
                    perf_editor_paint_us as f64 / 1_000.0,
                    perf_paint_chrome_us as f64 / 1_000.0,
                    perf_chrome_drain_us as f64 / 1_000.0,
                    resources.last_surface_report(),
                    vertices.len(),
                ),
            );
            if let Some(event_loop_proxy) = self.platform.event_loop_proxy() {
                let _ = event_loop_proxy
                    .send_event(crate::app_event::AppEvent::StartBackgroundServices);
            }
        }

        let _total_render_us = _render_t0.elapsed().as_micros();
        if should_log_perf(_total_render_us) {
            let plugin = self.active_plugin_name().unwrap_or("unknown");
            eprintln!(
                "[perf:app_render] plugin={plugin} total={_total_render_us}us sync_prepare={perf_sync_prepare_us}us editor_paint={perf_editor_paint_us}us chrome_drain={perf_chrome_drain_us}us gpu_submit={perf_gpu_submit_us}us vertices={}",
                vertices.len(),
            );
        }
        #[cfg(debug_assertions)]
        if _total_render_us > 1000 || _frame_interval_us > 20000 {
            let scroll_y =
                self.active_tab_session().map(|tab| tab.scroll_top() as usize).unwrap_or(0);
            let vis = self.active_tab_session().map(|tab| tab.visible_rows()).unwrap_or(0);
            eprintln!(
                "[frame] total={:.0}us interval={:.0}us scroll_y={} visible={}",
                _total_render_us, _frame_interval_us, scroll_y, vis,
            );
        }

        #[cfg(debug_assertions)]
        {
            let render_frame_count = self.editor_runtime.render_frame_count();
            if render_frame_count.is_multiple_of(60) {
                eprintln!(
                    "[perf] frame#{} total={}us interval={}us",
                    render_frame_count, _total_render_us, _frame_interval_us,
                );
            }
        }

        // ── Atlas exhaustion recovery ──
        if resources.text.as_ref().is_some_and(|t| t.atlas.allocation_failed) {
            if let Some(text) = resources.text.as_mut() {
                text.atlas.clear();
                text.preview_cache.invalidate_all();
            }
            let tab_ids = self.editor_tab_ids_in_order();
            for tab_id in tab_ids {
                if let Some(mut tab) = self.tab_session_mut(tab_id) {
                    tab.invalidate_render_cache_all();
                }
            }
            self.platform.schedule_redraw();
        }

        if self.editor_runtime.take_redraw_request() {
            self.platform.schedule_redraw();
        }

        if self.ui_shell.advance_tab_scroll() {
            self.platform.schedule_redraw();
        }

        if needs_redraw_after_render(
            self.platform.redraw_pending(),
            self.ui_shell.scrollbar_is_dragging(),
        ) {
            self.platform.schedule_redraw();
        }
        FrameRenderOutcome::Rendered(submission)
    }

    /// 用最新的 Shell 输入完成一次布局。
    fn update_shell_layout(&mut self, frame: &mut EditorFrame) {
        // Phase 3：注入 status_bar 数据，然后更新 dock 布局
        {
            use ui::core::Screen;
            use ui::search_bar::SearchBarSnapshot;
            use ui::status_bar::StatusBarInput;
            let metrics = self.ui_metrics();
            let screen = Screen::new(self.screen_width(), self.screen_height());

            let is_plugin_rendered = self.active_handles_own_rendering();
            // Phase 5: extract scrollbar input before borrowing dv
            let scrollbar_input = if is_plugin_rendered {
                self.active_tab_session().map(|v| {
                    let lh = metrics.line_height;
                    let vh = self.ui_shell.editor_rect().h;
                    let content_h = v.content_height();
                    let scroll_y = v.scroll_y();
                    let total = (content_h / lh).ceil() as usize;
                    let scroll_rows = scroll_y / lh;
                    ((vh / lh) as f64, total, scroll_rows as f64)
                })
            } else {
                self.active_tab_session()
                    .map(|v| (v.viewport_height(), v.total_display_rows(), v.scroll_top()))
            };
            let conflict_label = self.file_safety_status_label();
            let active_shell_input = self.active_tab_session().map(|entry| {
                let document = entry.document();
                let status = StatusBarInput {
                    buffer_len: document.buffer_len(),
                    selection_range: document.selection_range(),
                    selection_char_count: document.count_selection_chars(),
                    cursor_line: document.cursor_line(),
                    cursor_col: document.cursor_column(),
                    conflict_label,
                };
                let search = entry.search_state();
                let search_input = SearchBarSnapshot {
                    query: search.query.clone(),
                    match_count: search.matches.len(),
                    current_match: search.active_match_idx,
                    visible: search.panel_visible,

                    replace_query: search.replace_query.clone(),
                    replace_mode: search.replace_mode,
                    focus_replace: search.focus_replace,
                    options_use_regex: search.options.use_regex,
                };
                (status, search_input)
            });
            if let Some((status, search_input)) = active_shell_input {
                self.ui_shell.set_status_input(status);
                self.ui_shell.set_search_input(search_input);
                if let Some((vh, total, st)) = scrollbar_input {
                    self.ui_shell.set_scrollbar_input(vh, total, st);
                }
            }

            let inspector_panel = self.editor_runtime.active_inspector_panel();
            // Phase 6：注入 tab bar 数据
            {
                let tab_infos: Vec<ui::tab_bar::TabInfo> = self
                    .editor_tab_ids_in_order()
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, tab_id)| {
                        let save_context = self.editor_save_context(tab_id)?;
                        Some(ui::tab_bar::TabInfo {
                            title: save_context.title,
                            file_path: save_context.file_path,
                            is_dirty: save_context.dirty,
                            pinned: self.is_editor_tab_pinned_at(index),
                            language: String::new(),
                        })
                    })
                    .collect();
                let active_tab_index = self.active_editor_index();
                let has_back_history = self.editor_has_back_history();
                let has_forward_history = self.editor_has_forward_history();
                self.ui_shell.set_tabs_input(
                    tab_infos.clone(),
                    active_tab_index,
                    has_back_history,
                    has_forward_history,
                    self.ui_shell.tab_bar_hovered_index(),
                    self.ui_shell.animated_tab_scroll_offset(),
                );

                let dpi = metrics.dpi;
                let fullscreen = cfg!(target_os = "macos")
                    && self
                        .editor_runtime
                        .window()
                        .is_some_and(|window| window.fullscreen().is_some());
                let header_layout = sidebar_header_layout(fullscreen);
                self.ui_shell.set_sidebar_input(
                    self.ui_shell.sidebar_cfg().clone(),
                    tab_infos,
                    active_tab_index,
                    header_layout,
                );

                let file_path = self.active_editor_file_path();
                let toggle_target = self.active_editor_toggle_target();
                let can_toggle = toggle_target.is_some();
                let toggled = self.active_is_toggled();
                let toggle_label = toggle_target.map(|name| match name {
                    PLUGIN_EDITOR => "基础编辑".to_string(),
                    PLUGIN_NOVEL_VIEW => "小说模式".to_string(),
                    PLUGIN_MARKDOWN_EDITOR => "MD编辑".to_string(),
                    _ => name.to_string(),
                });
                let sidebar_mode =
                    self.editor_runtime.settings().view_mode == ui::view_mode::ViewMode::Sidebar;
                let sidebar_left = if sidebar_mode {
                    self.ui_shell
                        .sidebar_editor_left_offset(dpi)
                        .max(header_layout.reserved_titlebar_width_px(dpi))
                } else if cfg!(target_os = "macos") {
                    header_layout.toggle_left_px(dpi)
                } else {
                    0.0
                };
                let titlebar_x = if sidebar_mode {
                    self.ui_shell.sidebar_editor_left_offset(metrics.dpi).max(0.5)
                } else {
                    0.0
                };
                let toc_enabled = is_plugin_rendered;
                let inspector = self.editor_runtime.active_inspector_available().then_some(
                    ui::title_bar::InspectorButtonInput {
                        panel_visible: inspector_panel.is_some(),
                    },
                );
                self.ui_shell.set_title_bar_input(ui::title_bar::TitleBarInput {
                    file_path,
                    sidebar_left,
                    titlebar_x,
                    can_toggle,
                    toggled,
                    toggle_label,
                    toc_visible: self.active_toc_visible(),
                    toc_enabled,
                    inspector,
                    window_chrome: if cfg!(target_os = "windows") {
                        ui::title_bar::WindowChrome::Custom {
                            maximized: self
                                .editor_runtime
                                .window()
                                .is_some_and(|window| window.is_maximized()),
                        }
                    } else {
                        ui::title_bar::WindowChrome::Native
                    },
                    show_hamburger: !sidebar_mode,
                });
            }

            let inputs = self.build_shell_inputs();
            self.ui_shell.set_inspector_panel(inspector_panel, metrics.dpi);

            // Update TOC input before update_frame so dock rebuild has fresh data
            if self.active_toc_visible() {
                let session = self.active_tab_session().expect("visible TOC needs active tab");
                let outline = session.plugin().outline();
                let headings = outline.map_or_else(Vec::new, |outline| outline.headings());
                let active_idx = outline
                    .and_then(|outline| outline.current_heading_index(session.scroll_y()))
                    .unwrap_or(0);
                let toc_input = ui::toc::TocInput {
                    headings: headings
                        .iter()
                        .map(|h| ui::toc::TocHeadingEntry { text: h.title.clone(), level: h.level })
                        .collect(),
                    active_index: Some(active_idx),
                };
                self.ui_shell.set_toc_input(toc_input);
            }

            #[cfg(feature = "sync")]
            self.refresh_sync_settings_overlay();

            frame.with_layout_context(|context| {
                self.ui_shell.update_frame(screen, context.theme, context.measure, &inputs);
            });
            // The first layout defers sidebar creation until the native window is ready.
            if !self.editor_runtime.first_frame_presented() && self.ui_shell.dock_is_dirty() {
                self.platform.schedule_redraw();
            }
        }
    }

    /// 先同步活动插件的源文本，再完成画布 Prepare → Resolve。
    pub(crate) fn sync_and_prepare_canvas_frame(
        &mut self,
        shaper: &mut shaping::Shaper,
    ) -> Option<CanvasViewportSnapshot> {
        self.sync_plugin_state();
        self.prepare_active_canvas_frame(shaper)
    }

    /// 同步内容，先解析画布滚动条，再将覆盖层和编辑区布局到同一帧。
    fn prepare_editor_frame(
        &mut self,
        frame: &mut EditorFrame,
        shaper: &mut shaping::Shaper,
    ) -> Option<CanvasViewportSnapshot> {
        let canvas = self.sync_and_prepare_canvas_frame(shaper);
        self.update_shell_layout(frame);
        canvas
    }

    #[cfg(test)]
    pub(crate) fn prepare_editor_pointer_frame_for_test(&mut self) {
        let mut shaper = shaping::Shaper::new().expect("指针测试需要文字整形器");
        let mut frame = self.editor_runtime.begin_frame().expect("应能创建无窗口帧");
        let canvas = self.prepare_editor_frame(&mut frame, &mut shaper);
        self.paint_editor_surface(&mut frame, &mut RenderResources::default(), canvas.as_ref())
            .expect("应能发布活动编辑区的指针坐标");
    }

    fn paint_editor_surface(
        &mut self,
        frame: &mut EditorFrame,
        resources: &mut RenderResources,
        canvas: Option<&CanvasViewportSnapshot>,
    ) -> Option<()> {
        let handles_own_rendering = self.active_handles_own_rendering();
        if !handles_own_rendering {
            self.refresh_active_source_spacing_state();
        }
        let tab = self.active_tab_session()?;
        let search_has_focus =
            tab.search_state().panel_visible && self.ui_shell.search_bar_has_keyboard_focus();
        let editor_rect = self.ui_shell.editor_rect();
        let plugin_content = handles_own_rendering.then(|| {
            canvas.map_or_else(
                || PreparedPluginContent::Bounds(self.plugin_render_bounds()),
                PreparedPluginContent::Canvas,
            )
        });
        let options = EditorPaintOptions {
            plugin_content,
            text_left_margin: Some(
                self.toc_left_offset() + self.editor_left_margin(tab.document().line_count()),
            ),
            gutter_background: true,
            search_highlights: SearchHighlightStyle::Theme,
            text_highlight_layer: TextHighlightLayer::BelowGlyphs,
            search_caret_opacity: search_has_focus.then_some(SEARCH_FOCUSED_CARET_OPACITY),
            preedit_advance: PreeditAdvanceMeasure::AtLeastOnePixel,
            preedit_fallback_origin: Some((
                self.toc_left_offset() + self.editor_left_margin(0),
                self.content_top_offset(),
            )),
            caret_reveal: TextCaretReveal::SourceLine,
            reshape_ahead_width: (!handles_own_rendering)
                .then(|| self.viewport_content_width(tab.document())),
        };
        self.editor_runtime
            .paint_active_editor_with(frame, resources, editor_rect, options)
            .ok()?;
        Some(())
    }

    /// 为活动画布执行 Prepare → Resolve，并把本帧滚动条输入交给覆盖层。
    ///
    /// `None` 表示插件不是画布或没有准备画布指标，调用方必须回退既有
    /// `render()` 路径。
    pub(crate) fn prepare_active_canvas_frame(
        &mut self,
        shaper: &mut shaping::Shaper,
    ) -> Option<CanvasViewportSnapshot> {
        let is_canvas = self.active_is_canvas();
        if !is_canvas {
            self.ui_shell.set_canvas_scrollbars_input(None);
            return None;
        }

        self.ui_shell.mark_dock_dirty();
        let bounds = self.plugin_render_bounds();
        let dpi = self.ui_metrics().dpi;
        let theme = self.editor_runtime.theme().clone();
        let prepared = (|| {
            let mut tab = self.active_tab_session_mut()?;
            let metrics = tab.prepare_canvas_plugin(&theme, shaper, dpi)?;
            let snapshot = tab.prepare_canvas_viewport(metrics, bounds, dpi)?;
            let scrollbars = tab.canvas_viewport_scrollbars_input();
            Some((snapshot, scrollbars))
        })();
        let Some((snapshot, scrollbars)) = prepared else {
            self.ui_shell.set_canvas_scrollbars_input(None);
            return None;
        };
        self.ui_shell.set_canvas_scrollbars_input(Some(
            ui::canvas_scrollbars::CanvasScrollbarsInput {
                horizontal: scrollbars.horizontal,
                vertical: scrollbars.vertical,
            },
        ));

        Some(snapshot)
    }

    /// Synchronize the plugin's source text and cursor byte position
    /// with the current gap buffer state. Called before hit-testing and after edits.
    pub(crate) fn sync_plugin_state(&mut self) {
        let sync_started_at = Instant::now();
        let (preedit_text, preedit_cursor) = self.editor_runtime.preedit();
        let preedit_len = preedit_text.len();
        let mouse_is_down = self.editor_runtime.pointer_capture()
            != appkit_shell::editor_runtime::MouseCapture::None;
        let should_sync_selection = !mouse_is_down;
        let Some(mut tab) = self.active_tab_session_mut() else {
            return;
        };
        let should_log_wysiwyg_cursor = wysiwyg_cursor_logging_enabled();
        let plugin_name = if perf_logging_enabled() || should_log_wysiwyg_cursor {
            tab.plugin_name().to_string()
        } else {
            String::new()
        };

        let generation = tab.document().generation();
        let document_sync_started_at = Instant::now();
        let source_updated =
            tab.sync_document(ui::plugin::SyncReason::Normal) == ui::plugin::SyncOutcome::Updated;
        let document_sync_us = document_sync_started_at.elapsed().as_micros();
        if should_log_wysiwyg_cursor {
            eprintln!(
                "[wysiwyg:sync] begin plugin={} generation={} source_updated={} doc_cursor={} doc_selection={:?} mouse_down={} preedit_len={}",
                plugin_name,
                generation,
                source_updated,
                tab.document().cursor_offset().to_usize(),
                tab.document().selection_range(),
                mouse_is_down,
                preedit_len
            );
        }

        if should_sync_selection {
            if let Some((start, end)) = tab.document().selection_range() {
                if start < end {
                    tab.set_plugin_selection_anchor_byte(Some(start));
                    tab.set_plugin_selection_cursor_byte(Some(end));
                } else {
                    tab.set_plugin_selection_anchor_byte(None);
                    tab.set_plugin_selection_cursor_byte(None);
                }
            } else if plugin_selection_pull_is_safe(mouse_is_down, source_updated) {
                match tab.selection_byte_range() {
                    Some((start, end)) if start != end => {
                        if should_log_wysiwyg_cursor {
                            eprintln!(
                                "[wysiwyg:sync] pull_plugin_selection start={} end={} doc_len={}",
                                start,
                                end,
                                tab.document().buffer_len()
                            );
                        }
                        let selection_start = start.min(end).min(tab.document().buffer_len());
                        let selection_end = start.max(end).min(tab.document().buffer_len());
                        tab.document_mut().cursor_mut().selection_anchor = Some(selection_start);
                        tab.document_mut().set_cursor_offset_synced(selection_end);
                    }
                    None | Some(_) => {}
                }
            } else {
                tab.set_plugin_selection_anchor_byte(None);
                tab.set_plugin_selection_cursor_byte(None);
            }
        }

        let cursor_byte = tab.document().cursor_offset().to_usize();
        tab.set_cursor_byte(cursor_byte);
        let affinity = tab.document().tb().cursor_affinity();
        tab.set_visual_caret(ui::plugin::VisualCaretPosition {
            byte_offset: cursor_byte,
            affinity,
        });
        tab.set_preedit(preedit_text, preedit_cursor);
        if should_log_wysiwyg_cursor {
            eprintln!(
                "[wysiwyg:sync] end plugin={} cursor={} selection={:?}",
                plugin_name,
                tab.document().cursor_offset().to_usize(),
                tab.document().selection_range()
            );
        }

        let elapsed_us = sync_started_at.elapsed().as_micros();
        if should_log_perf(elapsed_us) {
            eprintln!(
                "[perf:plugin_sync] plugin={} total={}us source_updated={} document_sync={}us cursor_byte={} selection_sync={} preedit_len={}",
                plugin_name,
                elapsed_us,
                source_updated,
                document_sync_us,
                cursor_byte,
                should_sync_selection,
                preedit_len,
            );
        }
    }
}

#[cfg(test)]
mod toc_left_offset_tests {
    use super::*;
    use crate::document_fixture::DocumentFixture;
    use crate::plugins::editor::EditorPlugin;

    #[test]
    fn toc_left_offset_returns_zero_when_toc_hidden() {
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentFixture::new(vec!["# Hello".into()], 80, 10.0),
            Box::new(EditorPlugin::new()),
        );
        app.switch_workspace_for_test(0);
        // Default: toc_visible = false
        assert_eq!(app.toc_left_offset(), 0.0);
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn toc_left_offset_returns_only_gap_when_toc_visible() {
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentFixture::new(vec!["# Hello".into()], 80, 10.0),
            Box::new(textora_markdown::view::MarkdownEditorView::new()),
        );
        app.switch_workspace_for_test(0);
        app.active_tab_session_mut().expect("active Markdown tab").toggle_toc_visible();
        let metrics = app.ui_metrics();
        let expected_gap = 10.0 * metrics.dpi;
        // Must NOT include toc_width (the dock allocates that).
        assert_eq!(
            app.toc_left_offset(),
            expected_gap,
            "should return only the gap, not toc_width + gap"
        );
    }
}

#[cfg(test)]
mod file_safety_status_label_tests {
    use super::file_safety_notice_label;
    use appkit_core::file_safety::FileSafetyNotice;
    use std::path::PathBuf;

    #[test]
    fn maps_file_safety_notices_to_stable_labels() {
        assert_eq!(
            file_safety_notice_label(&FileSafetyNotice::ConflictCopyCreated {
                original: PathBuf::from("notes.md"),
                conflict: PathBuf::from("notes.textora-conflict.md"),
            }),
            "已保留冲突副本"
        );
        assert_eq!(
            file_safety_notice_label(&FileSafetyNotice::AmbiguousRename {
                original: PathBuf::from("notes.md"),
            }),
            "检测到歧义重命名"
        );
    }
}
