use std::collections::HashSet;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use appkit_shell::editor_runtime::{EditorSurfacePaint, RenderError, RenderResources};
use appkit_shell::render_state::{GpuState, TextState};
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

/// shell、主题、字体准备与 GPU frame 提交的唯一所有者。
pub(super) struct FrameRuntime {
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

    pub(super) fn synchronize_focus(&mut self, target: crate::FocusTarget, now: Instant) {
        self.shell.synchronize_focus(target, now);
    }

    pub(super) fn advance_text_cursor_blink(&mut self, now: Instant) -> bool {
        self.shell.advance_text_cursor_blink(now)
    }

    pub(super) fn next_text_cursor_blink_at(&self) -> Option<Instant> {
        self.shell.next_text_cursor_blink_at()
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
        input: FrameInput<'_>,
    ) -> Result<EditorSurfacePaint, RenderError> {
        let mut model =
            NotoraRenderModel::from_state_and_settings(input.state, input.product_settings);
        model.settings_overlay.persistence = input.persistence_view;
        model.editor_pane =
            if input.editor_is_active { EditorPaneState::Active } else { EditorPaneState::Empty };
        if input.editor_is_active
            && let Some(tab_id) = document_runtime.editor().active_tab_id()
        {
            Self::update_editor_render_model(document_runtime, &mut model, tab_id, input.layout);
        } else {
            model.editor_chrome = crate::editor_pane::EditorPaneInput::default();
        }
        let mut render_resources = document_runtime.editor_mut().take_render_resources();
        let mut frame = document_runtime.editor_mut().begin_frame()?;
        self.shell.render(&mut frame, input.layout, &model)?;
        let editor_surface = if input.editor_is_active {
            document_runtime.editor_mut().paint_active_editor(
                &mut frame,
                &mut render_resources,
                input.layout.editor_body_rect,
            )?
        } else {
            EditorSurfacePaint::Empty
        };
        let editor_scrollbars_input = (input.editor_is_active
            && input.state.layout.overlay == crate::OverlayState::None)
            .then(|| document_runtime.editor().active_editor_scrollbars_input())
            .flatten();
        frame.with_layout_context(|context| {
            self.shell.set_canvas_scrollbars_input(
                editor_scrollbars_input,
                input.layout.editor_body_rect,
                context,
            );
        });
        frame.with_paint_context(|context| self.shell.paint_canvas_scrollbars(context));
        let mut vertices = Vec::new();
        frame.drain_into(
            ui::Screen::new(input.window_width_px, input.window_height_px),
            &mut render_resources,
            &mut vertices,
        );
        submit_shell_frame(
            &mut render_resources,
            &vertices,
            self.theme.application_theme().editor_surface,
        );
        let _ = frame.present()?;
        document_runtime.editor_mut().restore_render_resources(render_resources);
        document_runtime.editor_mut().mark_frame_presented();
        Ok(editor_surface)
    }

    pub(super) fn update_focused_ime_cursor_area(
        &self,
        document_runtime: &DocumentRuntime,
        state: &NotoraState,
    ) {
        let ime_rect = self.shell.focused_text_input_ime_cursor_rect().or_else(|| {
            (state.layout.focus_target == crate::FocusTarget::Editor)
                .then(|| document_runtime.editor().active_editor_ime_cursor_rect())?
        });
        let Some(ime_rect) = ime_rect else {
            return;
        };
        let Some(window) = document_runtime.editor().window() else {
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
        tab_id: appkit_core::workspace::types::TabId,
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
        model.editor_chrome.header.compact = layout.editor_header_rect.h / layout.dpi
            < crate::shell::layout::EDITOR_COMPACT_HEIGHT_THRESHOLD_LOGICAL;
        if let Some(plugin_name) =
            document_runtime.editor().tab_session(tab_id).map(|tab| tab.plugin_name())
        {
            model.editor_chrome.toolbar = crate::render::editor_toolbar_input_for_plugin(
                model.editor_chrome.mode,
                plugin_name,
            );
            if document_runtime.editor().toggle_target().is_some() {
                let showing_source = document_runtime.editor().active_is_toggled(plugin_name);
                crate::render::add_source_toggle_command(
                    &mut model.editor_chrome.toolbar,
                    showing_source,
                );
            }
            if model.editor_chrome.header.compact {
                crate::render::add_compact_editor_toolbar_commands(
                    &mut model.editor_chrome.toolbar,
                );
            }
            if plugin_name == ui::plugin::PLUGIN_MINDMAP
                && let Some(tab) = document_runtime.editor().tab_session(tab_id)
            {
                let panel = tab.mindmap_style_panel();
                if panel.is_visible() {
                    model.mindmap_style_panel =
                        Some(ui::mindmap_style_panel::MindmapStylePanelInput::from_selection(
                            tab.mindmap_theme_selection(),
                            panel.presets_expanded(),
                        ));
                }
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

fn submit_shell_frame(
    render_resources: &mut RenderResources,
    vertices: &[render::GlyphVertex],
    background: [f32; 4],
) {
    let Some(gpu) = render_resources.gpu.as_mut() else {
        return;
    };
    let surface_texture = match gpu.ctx.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(texture)
        | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
        _ => return,
    };
    let surface_view = surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu.ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("notora shell render encoder"),
    });
    match render_resources.text.as_mut() {
        Some(text) if !vertices.is_empty() => {
            upload_shell_vertices(text, gpu, vertices);
            render_shell_vertices(
                &mut encoder,
                text,
                gpu,
                &surface_view,
                vertices.len(),
                background,
            );
        }
        _ => clear_shell_surface(&mut encoder, &surface_view, background),
    }
    gpu.ctx.queue.submit(std::iter::once(encoder.finish()));
    surface_texture.present();
}

fn upload_shell_vertices(text: &mut TextState, gpu: &GpuState, vertices: &[render::GlyphVertex]) {
    let vertex_count = vertices.len() as u32;
    if vertex_count > text.vertex_capacity {
        let capacity = vertex_count.next_power_of_two();
        text.vertex_buffer = gpu.ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("notora shell vertex buffer"),
            size: (capacity as usize * std::mem::size_of::<render::GlyphVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        text.vertex_capacity = capacity;
    }
    gpu.ctx.queue.write_buffer(&text.vertex_buffer, 0, bytemuck::cast_slice(vertices));
}

fn render_shell_vertices(
    encoder: &mut wgpu::CommandEncoder,
    text: &TextState,
    gpu: &GpuState,
    surface_view: &wgpu::TextureView,
    vertex_count: usize,
    background: [f32; 4],
) {
    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("notora shell text pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &gpu.ctx.msaa_view,
            depth_slice: None,
            resolve_target: Some(surface_view),
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(to_wgpu_color(background)),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        ..Default::default()
    });
    render_pass.set_pipeline(text.renderer.pipeline());
    render_pass.set_bind_group(0, &text.bind_group, &[]);
    render_pass.set_vertex_buffer(0, text.vertex_buffer.slice(..));
    render_pass.draw(0..vertex_count as u32, 0..1);
}

fn clear_shell_surface(
    encoder: &mut wgpu::CommandEncoder,
    surface_view: &wgpu::TextureView,
    background: [f32; 4],
) {
    let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("notora shell clear pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: surface_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(to_wgpu_color(background)),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        ..Default::default()
    });
}

fn to_wgpu_color(color: [f32; 4]) -> wgpu::Color {
    wgpu::Color { r: color[0] as f64, g: color[1] as f64, b: color[2] as f64, a: color[3] as f64 }
}
