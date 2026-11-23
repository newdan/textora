use std::collections::HashSet;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use appkit_shell::editor_runtime::{
    EditorFrame, EditorRuntime, EditorSurfacePaint, EmptyFrameTarget, RenderError, RenderResources,
    SurfaceFrameOptions, WindowCornerClip,
};
use appkit_shell::source_spacing;
use ui::Widget;
use ui::search_bar::{SearchBarEvent, SearchBarSnapshot, SearchBarWidget};
use winit::dpi::{PhysicalPosition, PhysicalSize};

use crate::NotoraState;
use crate::autosave::AutoSaveState;
use crate::render::{EditorPaneState, NotoraRenderModel, NotoraShell};
use crate::settings::ProductSettings;
use crate::settings_overlay::NotoraSettingsPersistenceView;
use crate::shell::layout::ShellLayout;

use super::document_runtime::DocumentRuntime;

const STARTUP_TRACE_ENVIRONMENT_VARIABLE: &str = "NOTORA_TRACE_STARTUP";
const SAVE_STATUS_SAVED: &str = "已保存";
const SAVE_STATUS_UNSAVED: &str = "未保存";
const SAVE_STATUS_PENDING: &str = "待保存";
const SAVE_STATUS_SAVING: &str = "保存中";
const SAVE_STATUS_FAILED: &str = "保存失败";

fn refresh_source_spacing_state(editor: &mut EditorRuntime) {
    let Some(tab_id) = editor.active_tab_id() else {
        return;
    };
    let Some(mut tab) = editor.tab_session_mut(tab_id) else {
        return;
    };
    source_spacing::refresh(&mut tab, Some(ui::plugin::PLUGIN_MARKDOWN_EDITOR), |document| {
        textora_markdown::typography::source_spacing_protected_ranges(&document.full_text())
    });
}

#[derive(Debug)]
pub(super) struct StartupTrace {
    pub(super) started_at: Instant,
    reported_milestones: HashSet<StartupMilestone>,
    restored_document_expected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum StartupMilestone {
    FirstFrameVisible,
    SessionRestoreStarted,
    WorkspaceSessionReady,
    SessionRestoreFinished,
    RestoredDocumentRendered,
}

pub(super) enum FontSystemPreparation {
    Deferred,
    InProgress(thread::JoinHandle<shaping::FontSystem>),
}

impl StartupTrace {
    pub(super) fn from_environment() -> Option<Self> {
        std::env::var_os(STARTUP_TRACE_ENVIRONMENT_VARIABLE).is_some().then(Self::started_now)
    }

    pub(super) fn started_now() -> Self {
        Self {
            started_at: Instant::now(),
            reported_milestones: HashSet::new(),
            restored_document_expected: false,
        }
    }

    pub(super) fn record_stage(&self, label: &str, stage_started_at: Instant) {
        eprintln!(
            "[startup] {label} stage={:.2}ms total={:.2}ms",
            stage_started_at.elapsed().as_secs_f64() * 1_000.0,
            self.started_at.elapsed().as_secs_f64() * 1_000.0,
        );
    }

    pub(super) fn take_first_frame_elapsed(&mut self) -> Option<Duration> {
        self.take_milestone_elapsed(StartupMilestone::FirstFrameVisible)
    }

    pub(super) fn take_milestone_elapsed(
        &mut self,
        milestone: StartupMilestone,
    ) -> Option<Duration> {
        self.reported_milestones.insert(milestone).then(|| self.started_at.elapsed())
    }
}

#[derive(Default, PartialEq, Eq)]
enum DocumentSearchComposition {
    #[default]
    Idle,
    Active,
    Cancelled,
}

/// shell、主题、字体准备与 GPU frame 提交的唯一所有者。
pub(super) struct FrameRuntime {
    pub(super) document_search_bar: SearchBarWidget,
    document_search_rect: ui::Rect,
    document_search_tab: Option<appkit_core::tab_set::types::TabId>,
    document_search_composition: DocumentSearchComposition,
    window_frame: ui::window_frame::WindowFrameWidget,
    #[cfg(not(test))]
    shell: NotoraShell,
    #[cfg(test)]
    pub(super) shell: NotoraShell,
    #[cfg(not(test))]
    settings: ui::Settings,
    #[cfg(test)]
    pub(super) settings: ui::Settings,
    #[cfg(not(test))]
    theme: ui::Theme,
    #[cfg(test)]
    pub(super) theme: ui::Theme,
    #[cfg(not(test))]
    startup_trace: Option<StartupTrace>,
    #[cfg(test)]
    pub(super) startup_trace: Option<StartupTrace>,
    #[cfg(not(test))]
    font_system_preparation: FontSystemPreparation,
    #[cfg(test)]
    pub(super) font_system_preparation: FontSystemPreparation,
}

pub(super) struct FrameInput<'a> {
    pub(super) state: &'a NotoraState,
    pub(super) product_settings: &'a ProductSettings,
    pub(super) persistence_view: NotoraSettingsPersistenceView,
    pub(super) layout: ShellLayout,
    pub(super) window_width_px: f32,
    pub(super) window_height_px: f32,
    pub(super) editor_is_active: bool,
}

impl FrameRuntime {
    pub(super) fn new(
        settings: ui::Settings,
        theme: ui::Theme,
        startup_trace: Option<StartupTrace>,
    ) -> Self {
        Self {
            document_search_bar: SearchBarWidget::find_only(),
            document_search_rect: ui::Rect::ZERO,
            document_search_tab: None,
            document_search_composition: DocumentSearchComposition::Idle,
            window_frame: ui::window_frame::WindowFrameWidget::default(),
            shell: NotoraShell::new(),
            settings,
            theme,
            startup_trace,
            font_system_preparation: FontSystemPreparation::Deferred,
        }
    }

    pub(super) fn record_startup_stage(&self, label: &str, started_at: Instant) {
        if let Some(trace) = &self.startup_trace {
            trace.record_stage(label, started_at);
        }
    }

    pub(super) fn record_application_constructed(&self) {
        if let Some(trace) = &self.startup_trace {
            trace.record_stage("application_constructed", trace.started_at);
        }
    }

    pub(super) fn focused_text_input_ime_allowed(&self) -> bool {
        if self.document_search_bar.input_has_focus() {
            return self.document_search_bar.ime_allowed();
        }
        self.shell.focused_text_input_ime_allowed()
    }

    pub(super) fn synchronize_focus(&mut self, target: crate::FocusTarget, now: Instant) {
        self.shell.synchronize_focus(target, now);
    }

    pub(super) fn advance_text_cursor_blink(&mut self, now: Instant) -> bool {
        let search_changed = self.document_search_bar.advance_cursor_blink(now);
        self.shell.advance_text_cursor_blink(now) || search_changed
    }

    pub(super) fn next_text_cursor_blink_at(&self) -> Option<Instant> {
        [self.shell.next_text_cursor_blink_at(), self.document_search_bar.next_cursor_blink_at()]
            .into_iter()
            .flatten()
            .min()
    }

    pub(super) fn synchronize_document_search(
        &mut self,
        tab: Option<appkit_core::tab_set::types::TabId>,
        input: SearchBarSnapshot,
        focused: bool,
        rect: ui::Rect,
        dpi: f32,
    ) {
        let binding_changed = self.document_search_tab != tab;
        if binding_changed || !focused {
            self.cancel_document_search_composition();
        }
        if binding_changed {
            self.document_search_bar = SearchBarWidget::find_only();
            self.document_search_tab = tab;
        }
        self.document_search_bar.set_input_with_focus(input, focused);
        if !binding_changed && self.document_search_rect == rect {
            return;
        }
        self.document_search_rect = rect;
        let mut measure = ui::NoopMeasure;
        let mut context =
            ui::LayoutCtx { ui_measure: None, measure: &mut measure, theme: &self.theme, dpi };
        self.document_search_bar.set_rect(rect, &mut context);
    }

    fn cancel_document_search_composition(&mut self) {
        if self.document_search_composition == DocumentSearchComposition::Active {
            self.document_search_composition = DocumentSearchComposition::Cancelled;
        }
    }

    pub(super) fn consumes_cancelled_search_ime(&mut self, event: &ui::Event) -> bool {
        match event {
            ui::Event::ImeCommit(_) => {
                std::mem::take(&mut self.document_search_composition)
                    == DocumentSearchComposition::Cancelled
            }
            ui::Event::ImePreedit { text, .. } if !text.is_empty() => {
                self.document_search_composition = DocumentSearchComposition::Idle;
                false
            }
            ui::Event::ImePreedit { .. } => {
                // 平台可在提交之前先清空预编辑，空预编辑不结束旧合成的归属。
                self.document_search_composition == DocumentSearchComposition::Cancelled
            }
            ui::Event::ImeDisable => {
                self.cancel_document_search_composition();
                false
            }
            _ => false,
        }
    }

    pub(super) fn route_document_search_event(
        &mut self,
        event: &ui::Event,
        dpi: f32,
    ) -> SearchBarEvent {
        let mut clipboard = appkit_shell::SystemClipboard;
        let mut context = ui::EventCtx::with_clipboard(&self.theme, dpi, &mut clipboard);
        let routed = self.document_search_bar.route_event(event, &mut context);
        if routed.consumed
            && matches!(event, ui::Event::ImePreedit { text, .. } if !text.is_empty())
        {
            self.document_search_composition = DocumentSearchComposition::Active;
        }
        routed
    }

    pub(super) fn editor_popup_is_open(&self) -> bool {
        self.shell.editor_popup_is_open()
    }

    pub(super) fn theme(&self) -> &ui::Theme {
        &self.theme
    }

    pub(super) fn settings(&self) -> &ui::Settings {
        &self.settings
    }

    pub(super) fn apply_product_settings(&mut self, product_settings: &ProductSettings) {
        product_settings.apply_to_ui(&mut self.settings);
    }

    pub(super) fn route_product_event(
        &mut self,
        event: &ui::Event,
        focus_target: crate::FocusTarget,
        overlay: crate::OverlayState,
        dpi: f32,
    ) -> crate::render::NotoraEventRoute {
        self.shell.route_event_with_overlay(event, focus_target, overlay, &self.theme, dpi)
    }

    pub(super) fn route_window_frame_event(
        &mut self,
        event: &ui::Event,
    ) -> ui::window_frame::WindowFrameEvent {
        self.window_frame.on_event(event)
    }

    pub(super) fn navigation_toggle_available(&self, overlay: crate::OverlayState) -> bool {
        self.shell.navigation_toggle_available(overlay)
    }

    pub(super) fn editor_title_text(&self) -> &str {
        self.shell.editor_title_text()
    }

    pub(super) fn start_font_system_preparation(&mut self, config_directory: &Path) {
        if matches!(self.font_system_preparation, FontSystemPreparation::InProgress(_)) {
            return;
        }
        let cache_path = config_directory.join("font-cache.bin");
        match thread::Builder::new()
            .name("notora-font-system-preparation".to_owned())
            .spawn(move || shaping::font_cache::new_font_system_with_cache(&cache_path))
        {
            Ok(worker) => {
                self.font_system_preparation = FontSystemPreparation::InProgress(worker);
            }
            Err(error) => {
                eprintln!(
                    "notora font preparation worker unavailable; falling back to synchronous initialization: {error}"
                );
            }
        }
    }

    pub(super) fn take_prepared_font_system(
        &mut self,
        config_directory: &Path,
    ) -> shaping::FontSystem {
        let preparation =
            std::mem::replace(&mut self.font_system_preparation, FontSystemPreparation::Deferred);
        if let FontSystemPreparation::InProgress(worker) = preparation
            && let Ok(font_system) = worker.join()
        {
            return font_system;
        }
        shaping::font_cache::new_font_system_with_cache(&config_directory.join("font-cache.bin"))
    }

    pub(super) fn record_first_frame_visible(&mut self) {
        let Some(trace) = self.startup_trace.as_mut() else {
            return;
        };
        let Some(elapsed) = trace.take_first_frame_elapsed() else {
            return;
        };
        eprintln!("[startup] first_frame_visible total={:.2}ms", elapsed.as_secs_f64() * 1_000.0,);
    }

    pub(super) fn record_session_restore_started(&mut self) {
        self.record_startup_milestone(StartupMilestone::SessionRestoreStarted);
    }

    pub(super) fn record_workspace_session_ready(&mut self, stage_started_at: Instant) {
        self.record_startup_stage_milestone(
            StartupMilestone::WorkspaceSessionReady,
            stage_started_at,
        );
    }

    pub(super) fn record_session_restore_finished(&mut self, stage_started_at: Instant) {
        self.record_startup_stage_milestone(
            StartupMilestone::SessionRestoreFinished,
            stage_started_at,
        );
    }

    pub(super) fn expect_restored_document_frame(&mut self, expected: bool) {
        if let Some(trace) = self.startup_trace.as_mut() {
            trace.restored_document_expected = expected;
        }
    }

    pub(super) fn record_restored_document_rendered(&mut self) {
        let Some(trace) = self.startup_trace.as_mut() else {
            return;
        };
        if !trace.restored_document_expected {
            return;
        }
        let Some(elapsed) =
            trace.take_milestone_elapsed(StartupMilestone::RestoredDocumentRendered)
        else {
            return;
        };
        eprintln!(
            "[startup] restored_document_rendered total={:.2}ms",
            elapsed.as_secs_f64() * 1_000.0,
        );
    }

    fn record_startup_milestone(&mut self, milestone: StartupMilestone) {
        let Some(trace) = self.startup_trace.as_mut() else {
            return;
        };
        let Some(elapsed) = trace.take_milestone_elapsed(milestone) else {
            return;
        };
        eprintln!(
            "[startup] {} total={:.2}ms",
            startup_milestone_label(milestone),
            elapsed.as_secs_f64() * 1_000.0,
        );
    }

    fn record_startup_stage_milestone(
        &mut self,
        milestone: StartupMilestone,
        stage_started_at: Instant,
    ) {
        let Some(trace) = self.startup_trace.as_mut() else {
            return;
        };
        let Some(elapsed) = trace.take_milestone_elapsed(milestone) else {
            return;
        };
        eprintln!(
            "[startup] {} stage={:.2}ms total={:.2}ms",
            startup_milestone_label(milestone),
            stage_started_at.elapsed().as_secs_f64() * 1_000.0,
            elapsed.as_secs_f64() * 1_000.0,
        );
    }

    pub(super) fn rebuild_theme(&mut self, mode: ui::ThemeMode, appearance: winit::window::Theme) {
        self.theme = ui::Theme::resolve_builtin(mode, appearance);
    }

    pub(super) fn render_frame(
        &mut self,
        document_runtime: &mut DocumentRuntime,
        mut input: FrameInput<'_>,
    ) -> Result<EditorSurfacePaint, RenderError> {
        let mut model =
            NotoraRenderModel::from_state_and_settings(input.state, input.product_settings);
        model.settings_overlay.persistence = std::mem::take(&mut input.persistence_view);
        model.editor_pane = if model.encrypted_note_unlock.is_some() {
            EditorPaneState::EncryptedUnlock
        } else if input.editor_is_active {
            EditorPaneState::Active
        } else {
            EditorPaneState::Empty
        };
        if input.editor_is_active
            && let Some(tab_id) = document_runtime.editor().active_tab_id()
        {
            Self::update_editor_render_model(document_runtime, &mut model, tab_id, input.layout);
        } else {
            model.editor_chrome = crate::editor_pane::EditorPaneInput::default();
        }
        refresh_source_spacing_state(document_runtime.editor_mut());
        let editor = document_runtime.editor_mut();
        let editor_surface = editor.with_render_resources(|editor, resources| {
            self.render_editor_frame(editor, resources, input, &model)
        })?;
        editor.mark_frame_presented();
        Ok(editor_surface)
    }

    fn render_editor_frame(
        &mut self,
        editor: &mut EditorRuntime,
        resources: &mut RenderResources,
        input: FrameInput<'_>,
        model: &NotoraRenderModel,
    ) -> Result<EditorSurfacePaint, RenderError> {
        let mut frame = editor.begin_frame()?;
        if let Some(text) = resources.text.as_mut() {
            text.begin_frame();
        }
        let frame_state = super::window_runtime::frame_state(editor.window());
        self.paint_frame_underlay(&mut frame, &input, frame_state);
        let editor_body_rect =
            crate::render::editor_inspector_rects(input.layout, model.inspector_panel.is_some()).0;
        let editor_surface = if input.editor_is_active {
            editor.paint_active_editor(&mut frame, resources, editor_body_rect)?
        } else {
            EditorSurfacePaint::Empty
        };
        let editor_scrollbars_input = (input.editor_is_active
            && input.state.layout.overlay == crate::OverlayState::None)
            .then(|| editor.active_editor_scrollbars_input())
            .flatten();
        frame.with_layout_context(|context| {
            self.shell.set_canvas_scrollbars_input(
                editor_scrollbars_input,
                editor_body_rect,
                context,
            );
        });
        let search_bar = (input.state.layout.overlay == crate::OverlayState::None)
            .then_some(&self.document_search_bar);
        self.shell.render_with_document_search(&mut frame, input.layout, model, search_bar)?;
        self.shell.synchronize_focus(input.state.layout.focus_target, Instant::now());
        frame.with_paint_context(|context| self.window_frame.paint_border(context));
        let mut vertices = Vec::new();
        frame.drain_into(
            ui::Screen::new(input.window_width_px, input.window_height_px),
            resources,
            &mut vertices,
        );
        let window_corner_radius = (frame_state == ui::window_frame::WindowFrameState::Restored)
            .then_some(ui::rounded_surface_frame::SHELL_CORNER_RADIUS_LOGICAL * input.layout.dpi);
        if let Err(error) = resources.submit_surface_frame(
            &vertices,
            SurfaceFrameOptions {
                background: self.theme.application_theme().editor_surface,
                empty_frame_target: EmptyFrameTarget::Surface,
                window_corner_clip: WindowCornerClip::Synchronize(window_corner_radius),
                gamma_override: None,
            },
        ) {
            eprintln!("[window] 无法同步合成窗口圆角：{error}");
        }
        let _ = frame.present()?;
        Ok(editor_surface)
    }

    fn paint_frame_underlay(
        &mut self,
        frame: &mut EditorFrame,
        input: &FrameInput<'_>,
        state: ui::window_frame::WindowFrameState,
    ) {
        frame.with_layout_context(|context| {
            self.document_search_bar.set_rect(self.document_search_rect, context);
        });
        self.window_frame.set_input(
            ui::window_frame::WindowFrameInput {
                title: super::PRODUCT_WINDOW_TITLE.to_owned(),
                state,
                navigation_toggle: if self.navigation_toggle_available(input.state.layout.overlay) {
                    ui::window_frame::WindowFrameNavigationToggle::Enabled
                } else {
                    ui::window_frame::WindowFrameNavigationToggle::Disabled
                },
            },
            ui::Rect::new(0.0, 0.0, input.window_width_px, input.window_height_px),
            input.layout.dpi,
        );
        frame.with_underlay_paint_context(|context| self.window_frame.paint_title(context));
    }

    pub(super) fn update_focused_ime_cursor_area(
        &self,
        document_runtime: &DocumentRuntime,
        state: &NotoraState,
    ) {
        let Some(window) = document_runtime.editor().window() else {
            return;
        };
        window.set_ime_allowed(self.focused_text_input_ime_allowed());
        let ime_rect = self
            .document_search_bar
            .ime_cursor_rect()
            .or_else(|| self.shell.focused_text_input_ime_cursor_rect())
            .or_else(|| {
                (state.layout.focus_target == crate::FocusTarget::Editor)
                    .then(|| document_runtime.editor().active_editor_ime_cursor_rect())?
            });
        let Some(ime_rect) = ime_rect else {
            return;
        };
        window.set_ime_cursor_area(
            PhysicalPosition::new(ime_rect.x as f64, (ime_rect.y + ime_rect.h) as f64),
            PhysicalSize::new(ime_rect.w.max(2.0) as f64, ime_rect.h as f64),
        );
    }

    pub(super) fn editor_save_status(
        state: Option<AutoSaveState>,
        dirty: bool,
        failure_message: Option<&str>,
    ) -> String {
        match state {
            Some(AutoSaveState::Saving { .. }) => SAVE_STATUS_SAVING.to_owned(),
            Some(AutoSaveState::Failed { .. }) => failure_message
                .map(|message| format!("{SAVE_STATUS_FAILED}：{message}"))
                .unwrap_or_else(|| SAVE_STATUS_FAILED.to_owned()),
            Some(AutoSaveState::Scheduled { .. }) => SAVE_STATUS_PENDING.to_owned(),
            Some(AutoSaveState::Idle) | None if dirty => SAVE_STATUS_UNSAVED.to_owned(),
            Some(AutoSaveState::Idle) | None => SAVE_STATUS_SAVED.to_owned(),
        }
    }

    fn update_editor_render_model(
        document_runtime: &DocumentRuntime,
        model: &mut NotoraRenderModel,
        tab_id: appkit_core::tab_set::types::TabId,
        layout: ShellLayout,
    ) {
        let Some(summary) = document_runtime.editor().document_summary(tab_id) else {
            model.editor_chrome = crate::editor_pane::EditorPaneInput::default();
            return;
        };
        model.editor_chrome.header.save_status_text = Self::editor_save_status(
            document_runtime.autosave_state(tab_id),
            summary.dirty,
            document_runtime.save_failure_message(tab_id),
        );
        model.editor_chrome.header.compact =
            editor_header_is_compact(layout, model.editor_chrome.mode.shows_property_row());
        if model.editor_pane == EditorPaneState::Active {
            model.inspector_panel = document_runtime.editor().active_inspector_panel();
        }
        if let Some(plugin_name) =
            document_runtime.editor().tab_session(tab_id).map(|tab| tab.plugin_name())
        {
            model.editor_chrome.toolbar = crate::render::editor_toolbar_input_for_plugin(
                model.editor_chrome.mode,
                plugin_name,
                document_runtime.editor().active_inspector_available(),
            );
            if document_runtime.editor().toggle_target().is_some() {
                let showing_source = document_runtime.editor().active_is_toggled(plugin_name);
                crate::render::add_source_toggle_command(
                    &mut model.editor_chrome.toolbar,
                    showing_source,
                );
            }
            if model.editor_chrome.header.compact && model.editor_chrome.header.delete_visible {
                crate::render::add_compact_editor_toolbar_commands(
                    &mut model.editor_chrome.toolbar,
                );
            }
            if plugin_name != ui::plugin::PLUGIN_MINDMAP {
                crate::render::add_document_search_command(&mut model.editor_chrome.toolbar);
            }
        }
    }
}

fn startup_milestone_label(milestone: StartupMilestone) -> &'static str {
    match milestone {
        StartupMilestone::FirstFrameVisible => "first_frame_visible",
        StartupMilestone::SessionRestoreStarted => "session_restore_started",
        StartupMilestone::WorkspaceSessionReady => "workspace_session_ready",
        StartupMilestone::SessionRestoreFinished => "session_restore_finished",
        StartupMilestone::RestoredDocumentRendered => "restored_document_rendered",
    }
}

fn editor_header_is_compact(layout: ShellLayout, property_row_visible: bool) -> bool {
    crate::editor_pane::compact_editor_header(
        layout.editor_header_rect,
        layout.dpi,
        property_row_visible,
    )
}

#[cfg(test)]
mod chrome_tests {
    use super::*;

    #[test]
    fn invalid_editor_rect_preserves_frame_resources_for_retry() {
        const CACHED_DOCUMENT_OFFSET: usize = 42;

        let mut app = super::super::tests::app();
        super::super::tests::install_registered_note(&mut app, "/frame-retry.md", "# Retry");
        app.document_runtime.editor_mut().with_render_resources(|_, resources| {
            resources.frame_cache.first_line.doc_offset = CACHED_DOCUMENT_OFFSET;
        });
        let valid_layout = app.shell_layout();
        let mut invalid_layout = valid_layout;
        invalid_layout.editor_body_rect.w = f32::NAN;
        let frame_input = |layout| FrameInput {
            state: app.action_runtime.state(),
            product_settings: app.persistence_runtime.product_settings(),
            persistence_view: app.persistence_runtime.persistence_view(),
            layout,
            window_width_px: 1200.0,
            window_height_px: 800.0,
            editor_is_active: true,
        };

        assert!(matches!(
            app.frame_runtime.render_frame(&mut app.document_runtime, frame_input(invalid_layout)),
            Err(RenderError::InvalidEditorRect)
        ));
        assert!(!app.document_runtime.editor().first_frame_presented());
        let preserved_offset = app
            .document_runtime
            .editor_mut()
            .with_render_resources(|_, resources| resources.frame_cache.first_line.doc_offset);
        assert_eq!(preserved_offset, CACHED_DOCUMENT_OFFSET);

        app.frame_runtime
            .render_frame(&mut app.document_runtime, frame_input(valid_layout))
            .expect("a valid frame should render after an invalid editor rect");
        assert!(app.document_runtime.editor().first_frame_presented());
    }

    #[test]
    fn wide_editor_does_not_duplicate_header_delete_in_toolbar() {
        let layout = ShellLayout {
            dpi: 1.0,
            editor_header_rect: ui::Rect::new(560.0, 0.0, 640.0, 84.0),
            ..ShellLayout::default()
        };
        assert!(!editor_header_is_compact(layout, true));
        let narrow =
            ShellLayout { editor_header_rect: ui::Rect::new(560.0, 0.0, 300.0, 84.0), ..layout };
        assert!(editor_header_is_compact(narrow, true));
    }
}

#[cfg(test)]
mod spacing_tests {
    #[test]
    fn untitled_markdown_keeps_code_protection_in_source_mode() {
        let mut app = super::super::tests::app();
        let editor = app.document_runtime.editor_mut();
        let (mut prepared, _) = crate::editor_adapter::prepare_untitled_document(
            editor,
            notora_core::DocumentKind::Markdown,
        )
        .expect("untitled Markdown should prepare");
        let source = "正文,继续 `代码,中文`";
        prepared.document.tb.write_raw(source.as_bytes());
        prepared.document.line_index =
            appkit_core::line_index::LineIndex::rebuild_from(&prepared.document.tb);
        prepared.document.mark_content_changed();
        editor.install_prepared_tab(
            prepared,
            None,
            appkit_shell::editor_runtime::OpenDisposition::Persistent,
        );
        let tab_id = editor.active_tab_id().expect("untitled tab should be active");
        editor
            .tab_session_mut(tab_id)
            .expect("untitled tab should exist")
            .replace_plugin(Box::new(appkit_shell::editor_plugin::EditorPlugin::new()));
        super::refresh_source_spacing_state(editor);
        let tab = editor.tab_session(tab_id).expect("source tab should exist");
        assert!(tab.document().file_path.is_none());
        assert_eq!(tab.document().full_text(), source);
        let source_spacing = tab.source_spacing();
        let code_start = source.find('`').expect("fixture has inline code");
        assert!(source_spacing.is_markdown);
        assert!(
            source_spacing
                .protected_ranges
                .iter()
                .any(|range| { range.start <= code_start && range.end >= source.len() })
        );
        assert!(!source_spacing.protected_ranges.iter().any(|range| range.contains(&0)));
    }
}
