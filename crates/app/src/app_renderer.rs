//! Vertex generation and GPU submission. Extracted from app.rs.
//! Methods here are part of `impl App`, just defined in a separate file.

use core::document::ReadableDocument;
use render::GlyphVertex;
use std::ops::Deref;
use std::sync::OnceLock;
use std::time::Instant;

use crate::app::App;
use crate::render_state::GpuState;
use appkit_shell::editor_runtime::{EditorFrame, RenderResources};
use ui::canvas::CanvasViewportSnapshot;
use ui::plugin::{PLUGIN_EDITOR, PLUGIN_MARKDOWN_EDITOR, PLUGIN_NOVEL_VIEW};

const PERF_LOG_ENV: &str = "EDIT_PLUS_PERF_LOG";
const PERF_LOG_THRESHOLD_US_ENV: &str = "EDIT_PLUS_PERF_LOG_THRESHOLD_US";
const DEFAULT_PERF_LOG_THRESHOLD_US: u128 = 1_000;
const WYSIWYG_CURSOR_LOG_ENV: &str = "EDIT_PLUS_WYSIWYG_CURSOR_LOG";

/// 已完成 Prepare、Shell 布局和画布渲染的正常帧结果。
///
/// 保留快照供调用方和测试检查，并携带尚待提交到 GPU 的绘制命令。
struct NormalCanvasFrame {
    snapshot: CanvasViewportSnapshot,
    draw_list: ui::core::paint::DrawList,
}

impl Deref for NormalCanvasFrame {
    type Target = CanvasViewportSnapshot;

    fn deref(&self) -> &Self::Target {
        &self.snapshot
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

fn plugin_selection_pull_is_safe(mouse_is_down: bool, plugin_needs_source_update: bool) -> bool {
    !mouse_is_down && !plugin_needs_source_update
}

fn active_mindmap_style_input(
    session: Option<crate::tab_session::TabSession<'_>>,
) -> Option<ui::mindmap_style_panel::MindmapStylePanelInput> {
    let session = session?;
    let panel = session.mindmap_style_panel();
    if session.plugin_name() != ui::plugin::PLUGIN_MINDMAP || !panel.is_visible() {
        return None;
    }

    let selection = session.mindmap_theme_selection();
    Some(ui::mindmap_style_panel::MindmapStylePanelInput::from_selection(
        selection,
        panel.presets_expanded(),
    ))
}

fn file_safety_notice_label(notice: &crate::file_safety::FileSafetyNotice) -> &'static str {
    match notice {
        crate::file_safety::FileSafetyNotice::CleanDocumentReloaded { .. } => "已同步远端修改",
        crate::file_safety::FileSafetyNotice::ConflictCopyCreated { .. } => "已保留冲突副本",
        crate::file_safety::FileSafetyNotice::DocumentDetachedAfterDeletion { .. } => {
            "文件已删除，内容已恢复"
        }
        crate::file_safety::FileSafetyNotice::ConflictCopyFailed { .. } => "冲突保护失败",
        crate::file_safety::FileSafetyNotice::AmbiguousRename { .. } => "检测到歧义重命名",
    }
}

#[cfg(test)]
mod tests {
    use appkit_core::workspace::types::TabIdAllocator;
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::{
        App, active_mindmap_style_input, needs_redraw_after_render, plugin_selection_pull_is_safe,
    };
    use crate::document_view::DocumentView;
    use crate::tab_runtime::TabRuntime;
    use ui::canvas::CanvasPoint;
    use ui::plugin::{
        CanvasContentMetrics, PluginMessage, PluginQuery, PluginResponse, ViewPlugin,
    };

    #[test]
    fn first_frame_queues_background_services_after_present() {
        let source = include_str!("app_renderer.rs");
        let present_position =
            source.find("output.present();").expect("renderer should present the frame");
        let startup_event = ["AppEvent::StartBackground", "Services"].concat();
        let startup_position =
            source.find(&startup_event).expect("renderer should queue deferred background startup");

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
    fn open_mmap_tab_builds_selected_theme_input() {
        let mut entry = theme_query_entry(
            ui::plugin::PLUGIN_MINDMAP,
            ui::theme::MindmapThemeSelection::Selected("tide".into()),
        );
        {
            let mut session = crate::tab_session::TabSessionMut::new(
                entry.id,
                &mut entry.document,
                &mut entry.runtime,
            );
            session.toggle_mindmap_style_panel();
        }

        let session =
            crate::tab_session::TabSession::new(entry.id, &entry.document, &entry.runtime);
        let input = active_mindmap_style_input(Some(session)).expect("open mmap panel input");

        assert!(input.options.iter().any(|option| option.id == "tide" && option.selected));
    }

    #[test]
    fn non_mmap_tab_hides_button_and_clears_right_panel() {
        let mut entry = theme_query_entry(
            ui::plugin::PLUGIN_EDITOR,
            ui::theme::MindmapThemeSelection::Selected("tide".into()),
        );
        {
            let mut session = crate::tab_session::TabSessionMut::new(
                entry.id,
                &mut entry.document,
                &mut entry.runtime,
            );
            session.toggle_mindmap_style_panel();
        }

        let session =
            crate::tab_session::TabSession::new(entry.id, &entry.document, &entry.runtime);
        assert!(active_mindmap_style_input(Some(session)).is_none());
    }

    #[test]
    fn switching_tabs_restores_each_style_panel_session() {
        let mut first = theme_query_entry(
            ui::plugin::PLUGIN_MINDMAP,
            ui::theme::MindmapThemeSelection::Default,
        );
        let second = theme_query_entry(
            ui::plugin::PLUGIN_MINDMAP,
            ui::theme::MindmapThemeSelection::Default,
        );
        {
            let mut session = crate::tab_session::TabSessionMut::new(
                first.id,
                &mut first.document,
                &mut first.runtime,
            );
            session.toggle_mindmap_style_panel();
        }

        let first_session =
            crate::tab_session::TabSession::new(first.id, &first.document, &first.runtime);
        let second_session =
            crate::tab_session::TabSession::new(second.id, &second.document, &second.runtime);
        assert!(active_mindmap_style_input(Some(first_session)).is_some());
        assert!(active_mindmap_style_input(Some(second_session)).is_none());
        let first_session =
            crate::tab_session::TabSession::new(first.id, &first.document, &first.runtime);
        assert!(active_mindmap_style_input(Some(first_session)).is_some());
    }

    #[test]
    fn canvas_prepare_resolves_viewport_before_render_canvas() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentView::new(vec!["canvas".to_string()], 10, 10.0),
            Box::new(CanvasSequencePlugin { events: Rc::clone(&events) }),
        );
        app.switch_workspace_for_test(0);
        let mut shaper = shaping::Shaper::new().expect("测试画布插件必须能够创建文本整形器");

        let snapshot = app
            .prepare_active_canvas_frame(&mut shaper)
            .expect("画布 Prepare 应生成可渲染的视口快照");
        let dpi = app.ui_metrics().dpi;
        let theme = app.current_theme.clone();
        let mut tab = app.active_tab_session_mut().expect("测试工作区必须保留活动画布标签");
        let _ = tab.render_canvas_plugin(&snapshot, &theme, &mut shaper, dpi);

        assert_eq!(&*events.borrow(), &["prepare", "render_canvas"]);
    }

    #[test]
    fn canvas_prepare_replaces_cached_legacy_scrollbar_bounds() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentView::new(vec!["canvas".to_string()], 10, 10.0),
            Box::new(CanvasSequencePlugin { events }),
        );
        app.switch_workspace_for_test(0);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
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
            &app.current_theme,
            &mut measure,
            &legacy_inputs,
        );
        app.ui_shell.update_frame(
            ui::core::Screen::new(app.screen_width(), app.screen_height()),
            &app.current_theme,
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
            DocumentView::new(vec![source.to_string()], 10, 10.0),
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
    fn normal_canvas_frame_layouts_synced_overlay_in_same_frame() {
        let state = Rc::new(RefCell::new(SourceSynchronizedCanvasState::new(200.0)));
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentView::new(vec!["canvas source".to_string()], 10, 10.0),
            Box::new(SourceSynchronizedCanvasPlugin { state: Rc::clone(&state) }),
        );
        app.switch_workspace_for_test(0);
        let mut shaper = shaping::Shaper::new().expect("测试画布插件必须能够创建文本整形器");

        let snapshot = app
            .run_normal_canvas_frame(&mut shaper)
            .expect("常规帧必须在单一调度入口中准备、布局并渲染画布");
        let editor_rect = app.ui_shell.editor_rect();
        let mut event_context = ui::core::widget::EventCtx {
            cursor_hint: None,
            theme: &app.current_theme,
            dpi: app.ui_metrics().dpi,
        };
        let action = app.ui_shell.dispatch(
            &ui::core::widget::Event::MouseDown {
                px: editor_rect.x + editor_rect.w * 0.5,
                py: editor_rect.bottom() - 1.0,
                button: ui::core::widget::MouseButton::Left,
            },
            &mut event_context,
        );
        assert_eq!(state.borrow().events, ["update_source", "prepare", "render_canvas"]);
        assert!(snapshot.max_scroll.x > 0.0, "测试夹具必须产生横向覆盖滚动条");
        assert!(
            matches!(action, Some(ui::core::widget::WidgetAction::CanvasScrollbars(_))),
            "同步后的覆盖滚动条必须在当前 Shell 布局帧命中"
        );
    }

    struct CanvasSequencePlugin {
        events: Rc<RefCell<Vec<&'static str>>>,
    }

    struct ThemeQueryPlugin {
        name: &'static str,
        selection: ui::theme::MindmapThemeSelection,
    }

    impl ViewPlugin for ThemeQueryPlugin {
        fn name(&self) -> &str {
            self.name
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

        fn query(&self, query: PluginQuery, _doc: &dyn core::document::DocView) -> PluginResponse {
            match query {
                PluginQuery::MindmapThemeSelection => {
                    PluginResponse::MindmapThemeSelection(self.selection.clone())
                }
                _ => PluginResponse::None,
            }
        }
    }

    struct ThemeQueryEntry {
        id: appkit_core::workspace::types::TabId,
        document: appkit_core::document::DocumentModel,
        runtime: TabRuntime,
    }

    fn theme_query_entry(
        name: &'static str,
        selection: ui::theme::MindmapThemeSelection,
    ) -> ThemeQueryEntry {
        let document = DocumentView::new(vec!["# Root".into()], 80, 10.0);
        let (document, presentation) = document.into_parts();
        ThemeQueryEntry {
            id: TabIdAllocator::new().allocate(),
            document,
            runtime: TabRuntime::with_presentation(
                Box::new(ThemeQueryPlugin { name, selection }),
                presentation,
            ),
        }
    }

    impl ViewPlugin for CanvasSequencePlugin {
        fn name(&self) -> &str {
            "canvas_frame_sequence_test"
        }

        fn render(
            &mut self,
            _doc: &dyn core::document::DocView,
            _bounds: ui::core::geom::Rect,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            self.events.borrow_mut().push("render");
            ui::core::paint::DrawList::new()
        }

        fn prepare_canvas(
            &mut self,
            _doc: &dyn core::document::DocView,
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
            _doc: &dyn core::document::DocView,
            _viewport: &ui::canvas::CanvasViewportSnapshot,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            self.events.borrow_mut().push("render_canvas");
            ui::core::paint::DrawList::new()
        }

        fn is_canvas(&self) -> bool {
            true
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

        fn prepare_canvas(
            &mut self,
            _doc: &dyn core::document::DocView,
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
            _doc: &dyn core::document::DocView,
            _viewport: &ui::canvas::CanvasViewportSnapshot,
            _theme: &ui::theme::Theme,
            _shaper: &mut shaping::Shaper,
            _dpi_scale: f32,
        ) -> ui::core::paint::DrawList {
            self.state.borrow_mut().events.push("render_canvas");
            ui::core::paint::DrawList::new()
        }

        fn handle_message(
            &mut self,
            message: PluginMessage,
            _doc: &mut dyn core::document::DocViewMut,
        ) -> bool {
            let PluginMessage::UpdateSource { text, generation } = message else {
                return false;
            };

            let mut state = self.state.borrow_mut();
            state.events.push("update_source");
            state.source = text;
            state.generation = Some(generation);
            true
        }

        fn query(&self, query: PluginQuery, _doc: &dyn core::document::DocView) -> PluginResponse {
            let PluginQuery::NeedsSourceUpdate(generation) = query else {
                return PluginResponse::None;
            };
            PluginResponse::Bool(self.state.borrow().generation != Some(generation))
        }

        fn is_canvas(&self) -> bool {
            true
        }
    }
}

/// Measure IME preedit text advance width in physical pixels.
/// Returns 0.0 when preedit_text is empty.
fn measure_preedit_advance_px(
    shaper: &mut shaping::Shaper,
    preedit_text: &str,
    font_size: f32,
) -> f32 {
    if preedit_text.is_empty() {
        return 0.0;
    }
    let old_font_size = shaper.font_size();
    shaper.set_font_size(font_size);
    let adv = shaper
        .shape(preedit_text)
        .map(|s| s.clusters.iter().map(|c| c.advance.max(1.0)).sum::<f32>())
        .unwrap_or(0.0);
    shaper.set_font_size(old_font_size);
    adv
}

impl App {
    fn file_safety_status_label(&self) -> Option<String> {
        self.file_safety_notices.last().map(file_safety_notice_label).map(str::to_owned)
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
        let gutter_w = self.settings.gutter_width(line_count) * metrics.dpi;
        let lm = metrics.content_left_margin.max(gutter_w);

        if matches!(self.settings.view_mode, ui::view_mode::ViewMode::Sidebar) {
            let sidebar_offset = self.ui_shell.sidebar_editor_left_offset();
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

    pub(crate) fn cursor_vertices(
        &self,
        screen_w: f32,
        screen_h: f32,
        tab_bar_height: f32,
    ) -> Vec<GlyphVertex> {
        let metrics = self.ui_metrics();
        let Some(tab) = self.active_tab_session() else {
            return vec![];
        };
        let sub_line_offset = tab.sub_line_pixel_offset(metrics.line_height);
        let search_has_focus =
            tab.search_state().panel_visible && self.ui_shell.search_bar_has_keyboard_focus();
        let dim_factor: Option<f32> = if search_has_focus { Some(0.4) } else { None };
        let cursor_x = tab.cursor_pixel_x() + self.preedit_advance_px;
        ui::decorations::cursor_vertices(
            &self.current_theme,
            tab.cursor_visual_line(),
            tab_bar_height,
            cursor_x,
            tab.cursor_blink_instant(),
            &metrics,
            screen_w,
            screen_h,
            sub_line_offset,
            dim_factor,
        )
    }

    pub(crate) fn gutter_bg_vertices(
        &self,
        left_margin: f32,
        screen_w: f32,
        screen_h: f32,
        tab_bar_height: f32,
    ) -> Vec<GlyphVertex> {
        let line_count = self.active_document_line_count();
        if line_count == 0 {
            return vec![];
        }
        let gutter_w =
            self.settings.gutter_width(line_count) * self.editor_runtime.scale_factor() as f32;
        if gutter_w <= 0.0 {
            return vec![];
        }
        let mut vertices = Vec::new();
        let bg_color = self.current_theme.editor.gutter_bg;
        let bg_left = ((left_margin - gutter_w) / screen_w * 2.0) - 1.0;
        let bg_right = (left_margin / screen_w * 2.0) - 1.0;
        let bg_top = 1.0 - (tab_bar_height / screen_h * 2.0);
        let bg_bottom = -1.0;
        let uv = 0.0;
        vertices.push(GlyphVertex {
            position: [bg_left, bg_top],
            tex_coords: [uv, uv],
            color: bg_color,
        });
        vertices.push(GlyphVertex {
            position: [bg_right, bg_top],
            tex_coords: [uv, uv],
            color: bg_color,
        });
        vertices.push(GlyphVertex {
            position: [bg_left, bg_bottom],
            tex_coords: [uv, uv],
            color: bg_color,
        });
        vertices.push(GlyphVertex {
            position: [bg_right, bg_top],
            tex_coords: [uv, uv],
            color: bg_color,
        });
        vertices.push(GlyphVertex {
            position: [bg_right, bg_bottom],
            tex_coords: [uv, uv],
            color: bg_color,
        });
        vertices.push(GlyphVertex {
            position: [bg_left, bg_bottom],
            tex_coords: [uv, uv],
            color: bg_color,
        });
        vertices
    }

    pub(crate) fn shape_visible_lines(
        &mut self,
        tab_bar_height: f32,
        tree_dirty: &mut bool,
        metrics: &ui::settings::UiMetrics,
        resources: &mut RenderResources,
    ) -> Vec<GlyphVertex> {
        let lc = self.active_document_line_count();
        let gutter_w = self.settings.gutter_width(lc) * metrics.dpi;
        let left_margin =
            self.toc_left_offset() + self.editor_left_margin_with_metrics(lc, metrics);
        let min_punct_ratio = self.settings.min_punctuation_width_ratio;

        let Some(tab_id) = self.active_tab_id() else {
            return vec![];
        };
        let App { editor_runtime, ui_shell, preedit_advance_px, current_theme, settings, .. } =
            self;
        let (Some(text), Some(gpu)) = (resources.text.as_mut(), resources.gpu.as_ref()) else {
            return vec![];
        };
        let (preedit_text, _) = editor_runtime.preedit();

        let screen_w = gpu.ctx.config.width as f32;
        let screen_h = gpu.ctx.config.height as f32;

        let Some(mut tab) = editor_runtime.tab_session_mut(tab_id) else {
            return vec![];
        };
        let search_has_focus =
            tab.search_state().panel_visible && ui_shell.search_bar_has_keyboard_focus();

        let (preedit_advance, preedit_cursor_col) = if !preedit_text.is_empty() && !search_has_focus
        {
            let font_size = metrics.font_size;
            let advance = measure_preedit_advance_px(&mut text.shaper, &preedit_text, font_size);
            *preedit_advance_px = advance;
            (advance, tab.document.cursor_column())
        } else {
            *preedit_advance_px = 0.0;
            (0.0, 0)
        };

        let context = ui::gutter::RenderContext {
            theme: current_theme,
            screen_w,
            screen_h,
            left_margin,
            tab_bar_height,
            is_active_tab: true,
            gutter_width: gutter_w,
            preedit_advance_px: preedit_advance,
            preedit_cursor_col,
        };

        let mut advance_cache = tab.take_advance_cache();
        let mut presentation = tab.take_presentation();
        let vertices = crate::render_pipeline::shape_visible_lines(
            metrics,
            min_punct_ratio,
            &context,
            tab.document,
            &mut presentation,
            text,
            gpu,
            &mut advance_cache,
            &mut resources.frame_cache.cluster_pool,
            &mut resources.frame_cache.first_line,
            &mut resources.frame_cache.last_line,
            tree_dirty,
            settings.word_wrap,
        );
        tab.restore_presentation(presentation);
        tab.restore_advance_cache(advance_cache);
        vertices
    }

    pub(crate) fn render(&mut self) -> Option<()> {
        let mut render_resources = self.editor_runtime.take_render_resources();
        let mut frame = self.editor_runtime.begin_frame().ok()?;
        let result = self.render_with_runtime_resources(&mut render_resources, &mut frame);
        let _frame_outcome = frame.present();
        self.editor_runtime.restore_render_resources(render_resources);
        result
    }

    fn render_with_runtime_resources(
        &mut self,
        resources: &mut RenderResources,
        frame: &mut EditorFrame,
    ) -> Option<()> {
        let _render_t0 = std::time::Instant::now();
        let _frame_interval_us = self.editor_runtime.note_render_started(_render_t0);
        if self.editor_is_empty() || self.active_editor_index().is_none() {
            return None;
        }
        self.needs_redraw = false;
        let screen_w = self.screen_width();
        let screen_h = self.screen_height();

        let metrics = self.ui_metrics();
        let mut perf_mode = "editor";
        let mut perf_plugin_name = String::new();
        let mut perf_plugin_setup_us = 0;
        let mut perf_plugin_render_us = 0;
        let mut perf_plugin_drain_us = 0;
        let mut perf_plugin_selection_us = 0;
        let mut perf_plugin_selection_drain_us = 0;
        let mut perf_plugin_search_us = 0;
        let mut perf_plugin_search_drain_us = 0;
        let mut perf_editor_reshape_us = 0;
        let mut perf_editor_shape_us = 0;
        let mut perf_editor_selection_us = 0;
        let mut perf_editor_search_us = 0;
        let mut perf_editor_cursor_us = 0;
        let sync_plugin_state_started_at = Instant::now();
        let mut text_state = resources.text.take();
        let normal_canvas_frame = if let Some(text) = text_state.as_mut() {
            let frame = self.run_normal_canvas_frame(&mut text.shaper);
            if frame.is_none() {
                self.update_shell_layout(&mut text.shaper);
            }
            frame
        } else {
            self.sync_plugin_state();
            self.ui_shell.set_canvas_scrollbars_input(None);
            None
        };
        let perf_sync_prepare_us = sync_plugin_state_started_at.elapsed().as_micros();
        resources.text = text_state;

        let content_top = self.content_top_offset();

        let mut vertices = Vec::new();
        let _editor_r = self.ui_shell.editor_rect();
        let line_count = self.active_document_line_count();
        let toc_off = self.toc_left_offset();
        let shows_gutter = self.active_shows_gutter();

        let gutter_left_margin =
            if line_count > 0 { toc_off + self.editor_left_margin(line_count) } else { 0.0 };

        if shows_gutter {
            vertices.extend(self.gutter_bg_vertices(
                gutter_left_margin,
                screen_w,
                screen_h,
                content_top,
            ));
        }

        // ── Plugin-rendered mode (MarkdownView, NovelView, Mindmap, WYSIWYG) ──
        if self.active_handles_own_rendering() || normal_canvas_frame.is_some() {
            perf_mode = "plugin";
            if perf_logging_enabled() {
                perf_plugin_name = self
                    .active_plugin_name()
                    .map(str::to_string)
                    .unwrap_or_else(|| "unknown".to_string());
            }
            let dpi = metrics.dpi;
            let bounds = self.plugin_render_bounds();

            let (dl, needs_drain) = if let Some(frame) = normal_canvas_frame {
                (frame.draw_list, true)
            } else {
                let handles_own_rendering = self.active_handles_own_rendering();
                let active_id = self.active_tab_id();
                let theme = self.current_theme.clone();
                let toc_max_depth = self.settings.toc_max_depth;
                let shaper = resources.text.as_mut().map(|text| &mut text.shaper);
                let session = active_id.and_then(|id| self.editor_runtime.tab_session_mut(id));
                if let Some(mut tab) = session
                    && handles_own_rendering
                {
                    let plugin_setup_started_at = Instant::now();
                    // Push render settings (logical pixels) before rendering
                    tab.send_message(ui::plugin::PluginMessage::SetRenderSettings {
                        font_size: metrics.font_size / dpi,
                        line_height: metrics.line_height / dpi,
                        toc_max_depth,
                    });
                    // Forward blink phase to all rendering plugins so they can hide/show cursor
                    let (visible, _) = crate::app::compute_cursor_phase(tab.cursor_blink_instant());
                    tab.send_message(ui::plugin::PluginMessage::SetCursorVisible(visible));
                    perf_plugin_setup_us = plugin_setup_started_at.elapsed().as_micros();
                    if let Some(shaper) = shaper {
                        let plugin_render_started_at = Instant::now();
                        let dl = tab.render_plugin(bounds, &theme, shaper, dpi);
                        perf_plugin_render_us = plugin_render_started_at.elapsed().as_micros();
                        (dl, true)
                    } else {
                        (ui::core::paint::DrawList::new(), true)
                    }
                } else {
                    (ui::core::paint::DrawList::new(), true)
                }
            };

            let screen = ui::core::Screen::new(screen_w, screen_h);
            let mut deferred_preview_verts: Option<Vec<render::GlyphVertex>> = None;
            if needs_drain {
                if let (Some(text), Some(gpu)) = (resources.text.as_mut(), resources.gpu.as_ref()) {
                    // Vertex caching handled by plugin internally
                    let plugin_drain_started_at = Instant::now();
                    deferred_preview_verts =
                        Some(crate::paint_backend::drain(dl, screen, Some(text), Some(gpu)));
                    perf_plugin_drain_us = plugin_drain_started_at.elapsed().as_micros();
                }
            } else {
                // Cached vertices handled by plugin internally
            }

            let mut deferred_sel_verts = None;
            let mut deferred_search_verts = None;

            // Render selection highlights (deferred until after preview content)
            if self.active_handles_own_rendering()
                && let Some(tab) = self.active_tab_session()
            {
                let selection_started_at = Instant::now();
                let has_sel = tab.has_selection();
                let sel_dl = if has_sel {
                    tab.selection_highlights(self.current_theme.editor.selection)
                } else {
                    ui::core::paint::DrawList::new()
                };
                perf_plugin_selection_us = selection_started_at.elapsed().as_micros();
                if !sel_dl.cmds.is_empty() {
                    let sel_drain_started_at = Instant::now();
                    let sel_verts = crate::paint_backend::drain(
                        sel_dl,
                        screen,
                        resources.text.as_mut().map(|t| &mut *t),
                        resources.gpu.as_ref(),
                    );
                    perf_plugin_selection_drain_us = sel_drain_started_at.elapsed().as_micros();

                    // We don't push to vertices here; we defer it until after preview_verts
                    deferred_sel_verts = Some(sel_verts);
                }
            }

            // Render search match highlights underneath preview content
            let mut search_info = None;
            if let Some(tab) = self.active_tab_session()
                && tab.search_state().is_active()
                && !tab.search_state().query.is_empty()
            {
                search_info = Some((
                    tab.search_state().query.clone(),
                    tab.search_state().options.match_case,
                    tab.search_state().options.use_regex,
                    tab.search_state().active_match_idx,
                ));
            }
            if let Some((query, match_case, use_regex, active_match_idx)) = search_info
                && let Some(tab) = self.active_tab_session()
            {
                let search_started_at = Instant::now();
                let search_dl = tab.search_highlights(
                    query.clone(),
                    match_case,
                    use_regex,
                    active_match_idx,
                    self.current_theme.palette.highlight,
                    self.current_theme.palette.inactive_highlight,
                );
                perf_plugin_search_us = search_started_at.elapsed().as_micros();
                let _ = (
                    &query,
                    match_case,
                    use_regex,
                    active_match_idx,
                    active_match_idx,
                    self.current_theme.palette.highlight,
                    self.current_theme.palette.inactive_highlight,
                );
                if !search_dl.cmds.is_empty() {
                    let search_drain_started_at = Instant::now();
                    let search_verts = crate::paint_backend::drain(
                        search_dl,
                        screen,
                        resources.text.as_mut().map(|t| &mut *t),
                        resources.gpu.as_ref(),
                    );
                    perf_plugin_search_drain_us = search_drain_started_at.elapsed().as_micros();
                    deferred_search_verts = Some(search_verts);
                }
            }

            // Now draw preview text content
            if let Some(preview_verts) = deferred_preview_verts {
                vertices.extend(preview_verts);
            }

            // Draw highlights ON TOP of the preview content so they are not
            // occluded by opaque block backgrounds (like CodeBlocks).
            if let Some(sel_verts) = deferred_sel_verts {
                vertices.extend(sel_verts);
            }
            if let Some(search_verts) = deferred_search_verts {
                vertices.extend(search_verts);
            }

            // Compute preedit advance so IME candidate window X tracks
            // the end of the composing text (shared with update_ime_cursor_area).
            let search_has_focus =
                self.active_tab_session().is_some_and(|tab| tab.search_state().panel_visible)
                    && self.ui_shell.search_bar_has_keyboard_focus();
            let (preedit_text, _) = self.editor_runtime.preedit();
            if !preedit_text.is_empty() && !search_has_focus {
                if let Some(text) = resources.text.as_mut() {
                    self.preedit_advance_px = measure_preedit_advance_px(
                        &mut text.shaper,
                        &preedit_text,
                        metrics.font_size,
                    );
                } else {
                    self.preedit_advance_px = 0.0;
                }
            }

            // Skip normal editor content; jump to chrome below
        } else {
            // Drain reshape worker results into DisplayLineMap before shaping
            let reshape_started_at = Instant::now();
            self.drain_reshape_results();
            perf_editor_reshape_us = reshape_started_at.elapsed().as_micros();

            let mut tree_dirty = false;
            let shape_started_at = Instant::now();
            let shape_verts =
                self.shape_visible_lines(content_top, &mut tree_dirty, &metrics, resources);
            perf_editor_shape_us = shape_started_at.elapsed().as_micros();

            if tree_dirty {
                if let Some(mut tab) = self.active_tab_session_mut() {
                    tab.derive_scroll_top(metrics.line_height);
                }
                self.needs_redraw = true;
            }
            // Selection highlight
            {
                let selection_started_at = Instant::now();
                if let Some(tab) = self.active_tab_session() {
                    let dv = tab.document;
                    let left_margin = toc_off + self.editor_left_margin(dv.line_count());
                    let sub_line_offset = tab.sub_line_pixel_offset(metrics.line_height);
                    let max_doc_line =
                        tab.advance_cache().iter().map(|e| e.doc_line).max().unwrap_or(0);
                    let mut line_offsets = vec![0usize; max_doc_line + 1];
                    let mut seen = std::collections::HashSet::new();
                    for entry in tab.advance_cache() {
                        let dl = entry.doc_line;
                        if seen.insert(dl) {
                            line_offsets[dl] = dv.line_byte_offset(dl).unwrap_or(0);
                        }
                    }
                    let verts = ui::decorations::selection_vertices(
                        dv.selection_range(),
                        tab.advance_cache(),
                        &metrics,
                        screen_w,
                        screen_h,
                        left_margin,
                        &self.current_theme,
                        content_top,
                        sub_line_offset,
                        &line_offsets,
                    );
                    vertices.extend(verts);
                }
                perf_editor_selection_us = selection_started_at.elapsed().as_micros();
            }

            // Search match highlights
            {
                let search_started_at = Instant::now();
                if let Some(tab) = self.active_tab_session()
                    && tab.search_state().is_active()
                {
                    let dv = tab.document;
                    let left_margin = toc_off + self.editor_left_margin(dv.line_count());
                    let sub_line_offset = tab.sub_line_pixel_offset(metrics.line_height);
                    let max_doc_line =
                        tab.advance_cache().iter().map(|e| e.doc_line).max().unwrap_or(0);
                    let mut line_offsets = vec![0usize; max_doc_line + 1];
                    let mut seen = std::collections::HashSet::new();
                    for entry in tab.advance_cache() {
                        let dl = entry.doc_line;
                        if seen.insert(dl) {
                            line_offsets[dl] = dv.line_byte_offset(dl).unwrap_or(0);
                        }
                    }
                    let verts = ui::decorations::search_match_vertices(
                        &tab.search_state()
                            .matches
                            .iter()
                            .map(|r| (r.start, r.end))
                            .collect::<Vec<_>>(),
                        tab.search_state().active_match_idx,
                        tab.search_state().is_active(),
                        tab.advance_cache(),
                        &metrics,
                        screen_w,
                        screen_h,
                        left_margin,
                        &self.current_theme,
                        content_top,
                        sub_line_offset,
                        &line_offsets,
                    );
                    vertices.extend(verts);
                }
                perf_editor_search_us = search_started_at.elapsed().as_micros();
            }

            vertices.extend(shape_verts);

            let cursor_started_at = Instant::now();
            vertices.extend(self.cursor_vertices(screen_w, screen_h, content_top));
            // IME preedit text rendering for document cursor
            let (preedit_text, _) = self.editor_runtime.preedit();
            if !preedit_text.is_empty() {
                let search_has_focus =
                    self.active_tab_session().is_some_and(|tab| tab.search_state().panel_visible)
                        && self.ui_shell.search_bar_has_keyboard_focus();

                if !search_has_focus {
                    let ime_fallback_x = toc_off + self.editor_left_margin(0);
                    let active_tab_id = self.active_tab_id();
                    if let (Some(text), Some(gpu)) =
                        (resources.text.as_mut(), resources.gpu.as_ref())
                        && let Some(tab) =
                            active_tab_id.and_then(|id| self.editor_runtime.tab_session(id))
                    {
                        if let Some(cursor_vl) = tab.cursor_visual_line() {
                            let line_height = metrics.line_height;
                            let sub_line_offset = tab.sub_line_pixel_offset(line_height);
                            let cursor_x = tab.cursor_pixel_x();
                            let cursor_y =
                                content_top + cursor_vl as f32 * line_height + sub_line_offset;
                            vertices.extend(crate::render_pipeline::preedit_text_vertices(
                                &metrics,
                                &preedit_text,
                                cursor_x,
                                cursor_y,
                                text,
                                gpu,
                                screen_w,
                                screen_h,
                                self.current_theme.editor.foreground,
                            ));
                        } else {
                            vertices.extend(crate::render_pipeline::preedit_text_vertices(
                                &metrics,
                                &preedit_text,
                                ime_fallback_x,
                                content_top,
                                text,
                                gpu,
                                screen_w,
                                screen_h,
                                self.current_theme.editor.foreground,
                            ));
                        }
                    }
                }
            }
            perf_editor_cursor_us = cursor_started_at.elapsed().as_micros();
        } // end else (normal editor mode)

        // Phase 3：用 ui_shell 渲染 chrome
        {
            let dpi = metrics.dpi;
            let chrome_list = self.ui_shell.paint_chrome(
                &self.current_theme,
                dpi,
                resources.text.as_mut().map(|t| &mut t.shaper),
            );
            frame.with_paint_context(|context| {
                context.list.cmds.extend(chrome_list.cmds);
            });
        }

        let editor_rect = self.ui_shell.editor_rect();
        frame.paint_editor_vertices(editor_rect, vertices.drain(..)).ok()?;
        let frame_drain_started_at = Instant::now();
        frame.drain_into(ui::core::Screen::new(screen_w, screen_h), resources, &mut vertices);
        let perf_chrome_drain_us = frame_drain_started_at.elapsed().as_micros();

        let gpu = resources.gpu.as_mut()?;
        let gpu: &mut GpuState = gpu;
        let output = match gpu.ctx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(tex)
            | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
            _ => return None,
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = gpu.ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render encoder"),
        });

        if let Some(text) = resources.text.as_mut() {
            if !vertices.is_empty() {
                let vertex_bytes = bytemuck::cast_slice(&vertices);
                let needed = vertices.len() as u32;

                if needed > text.vertex_capacity {
                    let new_cap = needed.next_power_of_two();
                    text.vertex_buffer = gpu.ctx.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("vertex buffer"),
                        size: (new_cap as usize * std::mem::size_of::<GlyphVertex>()) as u64,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    text.vertex_capacity = new_cap;
                }

                gpu.ctx.queue.write_buffer(&text.vertex_buffer, 0, vertex_bytes);

                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("text pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &gpu.ctx.msaa_view,
                        depth_slice: None,
                        resolve_target: Some(&view),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: self.current_theme.editor.background[0] as f64,
                                g: self.current_theme.editor.background[1] as f64,
                                b: self.current_theme.editor.background[2] as f64,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    ..Default::default()
                });

                render_pass.set_pipeline(text.renderer.pipeline());

                {
                    let bg = self.current_theme.editor.background;
                    let bg_brightness = 0.299 * bg[0] + 0.587 * bg[1] + 0.114 * bg[2];
                    let gamma = if bg_brightness < 0.5 { 2.2 } else { 1.45 };
                    text.update_gamma_if_changed(
                        &gpu.ctx.queue,
                        render::GammaUniform { contrast: 1.0, gamma },
                    );
                }

                render_pass.set_bind_group(0, &text.bind_group, &[]);
                render_pass.set_vertex_buffer(0, text.vertex_buffer.slice(..));
                render_pass.draw(0..vertices.len() as u32, 0..1);
            } else {
                let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("clear pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &gpu.ctx.msaa_view,
                        depth_slice: None,
                        resolve_target: Some(&view),
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: self.current_theme.editor.background[0] as f64,
                                g: self.current_theme.editor.background[1] as f64,
                                b: self.current_theme.editor.background[2] as f64,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    ..Default::default()
                });
            }
        } else {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.current_theme.editor.background[0] as f64,
                            g: self.current_theme.editor.background[1] as f64,
                            b: self.current_theme.editor.background[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
        }

        let gpu_submit_started_at = Instant::now();
        gpu.ctx.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        let perf_gpu_submit_us = gpu_submit_started_at.elapsed().as_micros();

        if !self.editor_runtime.first_frame_presented() {
            self.editor_runtime.mark_frame_presented();
            if let Some(w) = self.editor_runtime.window() {
                crate::sys::macos_titlebar::set_window_alpha(w, 1.0);
            }
            eprintln!(
                "[startup] first_frame_visible total: {:?}",
                self.startup_started_at.elapsed()
            );
            if let Some(event_loop_proxy) = self.event_loop_proxy.as_ref() {
                let _ = event_loop_proxy
                    .send_event(crate::app_event::AppEvent::StartBackgroundServices);
            }
        }

        let _total_render_us = _render_t0.elapsed().as_micros();
        if should_log_perf(_total_render_us) {
            if perf_mode == "plugin" {
                eprintln!(
                    "[perf:app_render] mode=plugin plugin={} total={}us sync_prepare={}us setup={}us plugin_render={}us plugin_drain={}us selection={}us selection_drain={}us search={}us search_drain={}us chrome_drain={}us gpu_submit={}us vertices={}",
                    perf_plugin_name,
                    _total_render_us,
                    perf_sync_prepare_us,
                    perf_plugin_setup_us,
                    perf_plugin_render_us,
                    perf_plugin_drain_us,
                    perf_plugin_selection_us,
                    perf_plugin_selection_drain_us,
                    perf_plugin_search_us,
                    perf_plugin_search_drain_us,
                    perf_chrome_drain_us,
                    perf_gpu_submit_us,
                    vertices.len(),
                );
            } else {
                eprintln!(
                    "[perf:app_render] mode=editor total={}us sync_prepare={}us reshape={}us shape={}us selection={}us search={}us cursor={}us chrome_drain={}us gpu_submit={}us vertices={}",
                    _total_render_us,
                    perf_sync_prepare_us,
                    perf_editor_reshape_us,
                    perf_editor_shape_us,
                    perf_editor_selection_us,
                    perf_editor_search_us,
                    perf_editor_cursor_us,
                    perf_chrome_drain_us,
                    perf_gpu_submit_us,
                    vertices.len(),
                );
            }
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

        let render_frame_count = self.editor_runtime.render_frame_count();
        #[cfg(debug_assertions)]
        if render_frame_count.is_multiple_of(60) {
            eprintln!(
                "[perf] frame#{} total={}us interval={}us",
                render_frame_count, _total_render_us, _frame_interval_us,
            );
        }

        // ── Atlas exhaustion recovery ──
        if resources.text.as_ref().is_some_and(|t| t.atlas.allocation_failed) {
            if let Some(text) = resources.text.as_mut() {
                text.atlas.clear();
                text.preview_cache.invalidate_all();
                text.atlas_generation += 1;
            }
            let tab_ids = self.editor_tab_ids_in_order();
            for tab_id in tab_ids {
                if let Some(mut tab) = self.tab_session_mut(tab_id) {
                    tab.invalidate_render_cache_all();
                }
            }
            self.needs_redraw = true;
        }

        self.post_shape_update();
        self.submit_reshape_ahead();

        // Sync animation target from TabBarWidget after render
        self.tab_scroll.set_target(self.ui_shell.tab_bar_scroll_target());

        if self.tab_scroll.tick() {
            self.needs_redraw = true;
        }

        self.needs_redraw =
            needs_redraw_after_render(self.needs_redraw, self.ui_shell.scrollbar_is_dragging());
        Some(())
    }

    /// 用最新的 Shell 输入完成一次布局。
    fn update_shell_layout(&mut self, shaper: &mut shaping::Shaper) {
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
            let (preedit_text, _) = self.editor_runtime.preedit();
            let active_shell_input = self.active_tab_session().map(|entry| {
                let document = entry.document;
                let status = StatusBarInput {
                    buffer_len: document.buffer_len(),
                    selection_range: document.selection_range(),
                    selection_char_count: document.count_selection_chars(),
                    cursor_line: document.cursor_line(),
                    cursor_col: document.cursor_column(),
                    conflict_label,
                };
                let search = entry.search_state();
                let (blink_on, _) = crate::app::compute_cursor_phase(entry.cursor_blink_instant());
                let search_input = SearchBarSnapshot {
                    query: search.query.clone(),
                    preedit_text: if search.panel_visible {
                        preedit_text.clone()
                    } else {
                        String::new()
                    },
                    match_count: search.matches.len(),
                    current_match: search.active_match_idx,
                    visible: search.panel_visible,

                    blink_on,
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
                    self.tab_scroll.current(),
                );

                let dpi = metrics.dpi;
                let traffic_inset = (68.0 * dpi, 0.0);
                self.ui_shell.set_sidebar_input(
                    self.ui_shell.sidebar_cfg().clone(),
                    tab_infos,
                    active_tab_index,
                    traffic_inset,
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
                let hamburger_right = ui::constants::TRAFFIC_LIGHT_TOTAL_W * dpi;
                let sidebar_left = self.ui_shell.sidebar_editor_left_offset().max(hamburger_right);
                let titlebar_x = self.ui_shell.sidebar_editor_left_offset().max(0.5);
                let toc_enabled = is_plugin_rendered;
                let active_is_mindmap = self.active_is_mindmap();
                let mindmap_style = self.active_tab_session().and_then(|session| {
                    active_is_mindmap.then_some(ui::title_bar::MindmapStyleButtonInput {
                        panel_visible: session.mindmap_style_panel().is_visible(),
                    })
                });
                self.ui_shell.set_title_bar_input(ui::title_bar::TitleBarInput {
                    file_path,
                    sidebar_left,
                    titlebar_x,
                    can_toggle,
                    toggled,
                    toggle_label,
                    toc_visible: self.active_toc_visible(),
                    toc_enabled,
                    mindmap_style,
                });
            }

            let inputs = self.build_shell_inputs();
            let mindmap_style_panel_input = active_mindmap_style_input(self.active_tab_session());
            self.ui_shell.set_mindmap_style_panel_input(mindmap_style_panel_input, metrics.dpi);

            // Update TOC input before update_frame so dock rebuild has fresh data
            if self.active_toc_visible() {
                let session = self.active_tab_session().expect("visible TOC needs active tab");
                let headings = match session.query(ui::plugin::PluginQuery::TOCHeadings) {
                    ui::plugin::PluginResponse::Headings(h) => h,
                    _ => vec![],
                };
                let scroll_y = match session.query(ui::plugin::PluginQuery::ScrollY) {
                    ui::plugin::PluginResponse::Float(v) => v,
                    _ => 0.0,
                };
                let active_idx =
                    match session.query(ui::plugin::PluginQuery::CurrentHeadingIndex(scroll_y)) {
                        ui::plugin::PluginResponse::Position(Some((i, _))) => i,
                        _ => 0,
                    };
                let toc_input = ui::toc::TocInput {
                    headings: headings
                        .iter()
                        .map(|h| ui::toc::TocHeadingEntry { text: h.title.clone(), level: h.level })
                        .collect(),
                    active_index: Some(active_idx),
                };
                self.ui_shell.set_toc_input(toc_input);
            }

            self.refresh_sync_settings_overlay();

            let mut measure = crate::measure_adapter::MeasureFromShaper(shaper);
            self.ui_shell.update_frame(screen, &self.current_theme, &mut measure, &inputs);
            if self.ui_shell.dock_is_dirty() {
                self.needs_redraw = true;
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

    /// 调度正常画布帧：同步→Prepare/Resolve→Shell 布局→Canvas Render。
    ///
    /// `None` 表示当前插件未能提供画布快照，调用方应继续使用既有渲染路径。
    fn run_normal_canvas_frame(
        &mut self,
        shaper: &mut shaping::Shaper,
    ) -> Option<NormalCanvasFrame> {
        let snapshot = self.sync_and_prepare_canvas_frame(shaper)?;
        self.update_shell_layout(shaper);

        let metrics = self.ui_metrics();
        let dpi = metrics.dpi;
        let font_size = metrics.font_size / dpi;
        let line_height = metrics.line_height / dpi;
        let toc_max_depth = self.settings.toc_max_depth;
        let theme = self.current_theme.clone();
        let mut tab = self.active_tab_session_mut()?;

        tab.send_message(ui::plugin::PluginMessage::SetRenderSettings {
            font_size,
            line_height,
            toc_max_depth,
        });
        let (cursor_visible, _) = crate::app::compute_cursor_phase(tab.cursor_blink_instant());
        tab.send_message(ui::plugin::PluginMessage::SetCursorVisible(cursor_visible));
        let draw_list = tab.render_canvas_plugin(&snapshot, &theme, shaper, dpi);

        Some(NormalCanvasFrame { snapshot, draw_list })
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
        let theme = self.current_theme.clone();
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
        let mut source_build_us = 0;
        let mut update_source_us = 0;
        let mut source_len = 0;
        let (preedit_text, preedit_cursor) = self.editor_runtime.preedit();
        let preedit_len = preedit_text.len();
        let should_sync_selection = !self.mouse.is_down;
        let mouse_is_down = self.mouse.is_down;
        let Some(mut tab) = self.active_tab_session_mut() else {
            return;
        };
        let should_log_wysiwyg_cursor = wysiwyg_cursor_logging_enabled();
        let plugin_name = if perf_logging_enabled() || should_log_wysiwyg_cursor {
            tab.plugin_name().to_string()
        } else {
            String::new()
        };

        let generation = tab.document.tb().gap_buffer().generation();
        let needs_update = tab.needs_source_update(generation);
        if should_log_wysiwyg_cursor {
            eprintln!(
                "[wysiwyg:sync] begin plugin={} generation={} needs_update={} doc_cursor={} doc_selection={:?} mouse_down={} preedit_len={}",
                plugin_name,
                generation,
                needs_update,
                tab.document.cursor_offset().to_usize(),
                tab.document.selection_range(),
                mouse_is_down,
                preedit_len
            );
        }

        if needs_update {
            let source_build_started_at = Instant::now();
            let gb = tab.document.tb().gap_buffer();
            let c1 = gb.read_forward(0);
            let c2 = gb.read_forward(c1.len());
            source_len = c1.len() + c2.len();
            let mut text = String::with_capacity(source_len);
            text.push_str(&String::from_utf8_lossy(c1));
            text.push_str(&String::from_utf8_lossy(c2));
            source_build_us = source_build_started_at.elapsed().as_micros();
            let update_source_started_at = Instant::now();
            tab.send_message(ui::plugin::PluginMessage::UpdateSource { text, generation });
            update_source_us = update_source_started_at.elapsed().as_micros();
        }

        if should_sync_selection {
            if let Some((start, end)) = tab.document.selection_range() {
                if start < end {
                    tab.send_message(ui::plugin::PluginMessage::SetSelAnchorByte(Some(start)));
                    tab.send_message(ui::plugin::PluginMessage::SetSelCursorByte(Some(end)));
                } else {
                    tab.send_message(ui::plugin::PluginMessage::SetSelAnchorByte(None));
                    tab.send_message(ui::plugin::PluginMessage::SetSelCursorByte(None));
                }
            } else if plugin_selection_pull_is_safe(mouse_is_down, needs_update) {
                match tab.selection_byte_range() {
                    Some((start, end)) if start != end => {
                        if should_log_wysiwyg_cursor {
                            eprintln!(
                                "[wysiwyg:sync] pull_plugin_selection start={} end={} doc_len={}",
                                start,
                                end,
                                tab.document.buffer_len()
                            );
                        }
                        let selection_start = start.min(end).min(tab.document.buffer_len());
                        let selection_end = start.max(end).min(tab.document.buffer_len());
                        tab.document.cursor_mut().selection_anchor = Some(selection_start);
                        tab.document.set_cursor_offset_synced(selection_end);
                    }
                    None | Some(_) => {}
                }
            } else {
                tab.send_message(ui::plugin::PluginMessage::SetSelAnchorByte(None));
                tab.send_message(ui::plugin::PluginMessage::SetSelCursorByte(None));
            }
        }

        let cursor_byte = tab.document.cursor_offset().to_usize();
        tab.send_message(ui::plugin::PluginMessage::SetCursorByte(cursor_byte));
        tab.send_message(ui::plugin::PluginMessage::SetPreedit {
            text: preedit_text,
            cursor: preedit_cursor,
        });
        if should_log_wysiwyg_cursor {
            eprintln!(
                "[wysiwyg:sync] end plugin={} cursor={} selection={:?}",
                plugin_name,
                tab.document.cursor_offset().to_usize(),
                tab.document.selection_range()
            );
        }

        let elapsed_us = sync_started_at.elapsed().as_micros();
        if should_log_perf(elapsed_us) {
            eprintln!(
                "[perf:plugin_sync] plugin={} total={}us needs_update={} source_build={}us update_source={}us source_len={} cursor_byte={} selection_sync={} preedit_len={}",
                plugin_name,
                elapsed_us,
                needs_update,
                source_build_us,
                update_source_us,
                source_len,
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
    use crate::document_view::DocumentView;
    use crate::plugins::editor::EditorPlugin;

    #[test]
    fn toc_left_offset_returns_zero_when_toc_hidden() {
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentView::new(vec!["# Hello".into()], 80, 10.0),
            Box::new(EditorPlugin::new()),
        );
        app.switch_workspace_for_test(0);
        // Default: toc_visible = false
        assert_eq!(app.toc_left_offset(), 0.0);
    }

    #[test]
    fn toc_left_offset_returns_only_gap_when_toc_visible() {
        let mut app = App::new(None);
        app.push_entry_for_test(
            DocumentView::new(vec!["# Hello".into()], 80, 10.0),
            Box::new(EditorPlugin::new()),
        );
        app.switch_workspace_for_test(0);
        app.active_tab_session_mut().unwrap().runtime.toc_visible = true;
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
    use crate::file_safety::FileSafetyNotice;
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
