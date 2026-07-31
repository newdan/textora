//! notora 窗口应用状态；编辑器会话只经 shared runtime 管理。

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use appkit_shell::editor_plugin::EditorPluginFactory;
use appkit_shell::editor_runtime::{
    EditorRuntime, EditorRuntimeConfig, EditorRuntimeError, RenderError, RenderResources,
};
use appkit_shell::render_state::{GpuState, TextState};
use appkit_shell::view_route::ViewRouteTable;
use appkit_shell::{ProductHost, ProductWakeHandle, ShellEffect, ShellEvent};
use ui::plugin::PLUGIN_EDITOR;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::WindowAttributes;

use crate::action::{CardQuery, NoteCreationTarget, NotoraAction};
use crate::effect_executor::{EffectExecutor, NotoraEffectService};
use crate::events;
use crate::product::NotoraProduct;
use crate::render::{NotoraRenderModel, NotoraShell};
use crate::shell::layout::{ShellLayout, ShellLayoutInput};
use crate::{NotoraPaths, NotoraPathsError, NotoraState};
use notora_core::{DocumentIdentity, DocumentKind};

const DEFAULT_WINDOW_WIDTH_PX: f32 = 1_200.0;
const DEFAULT_WINDOW_HEIGHT_PX: f32 = 800.0;

/// notora 应用初始化失败。
#[derive(Debug)]
pub enum NotoraAppError {
    Paths(NotoraPathsError),
    Runtime(EditorRuntimeError),
}

impl std::fmt::Display for NotoraAppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Paths(error) => write!(formatter, "could not initialize notora paths: {error}"),
            Self::Runtime(error) => {
                write!(formatter, "could not initialize editor runtime: {error}")
            }
        }
    }
}

impl std::error::Error for NotoraAppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Paths(error) => Some(error),
            Self::Runtime(error) => Some(error),
        }
    }
}

/// 组合产品状态、后台宿主和共享 editor runtime 的 notora 应用。
pub struct NotoraApp {
    paths: NotoraPaths,
    settings: ui::Settings,
    theme: ui::Theme,
    state: NotoraState,
    product: NotoraProduct,
    editor_runtime: EditorRuntime,
    shell: NotoraShell,
    window_focused: bool,
    window_width_px: f32,
    window_height_px: f32,
    pointer_position: (f32, f32),
    needs_redraw: bool,
    event_loop_proxy: Option<EventLoopProxy<ShellEvent>>,
}

impl NotoraApp {
    pub fn new() -> Self {
        Self::try_new()
            .expect("notora must construct its isolated configuration and editor runtime")
    }

    pub fn try_new() -> Result<Self, NotoraAppError> {
        let paths = NotoraPaths::from_platform_directory().map_err(NotoraAppError::Paths)?;
        Self::with_paths(paths)
    }

    pub fn with_paths(paths: NotoraPaths) -> Result<Self, NotoraAppError> {
        let settings = ui::Settings::new();
        let theme = ui::Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
        let editor_runtime = build_editor_runtime(&settings, &theme, &paths)?;
        Ok(Self {
            paths,
            settings,
            theme,
            state: NotoraState::default(),
            product: NotoraProduct::new(),
            editor_runtime,
            shell: NotoraShell::new(),
            window_focused: true,
            window_width_px: DEFAULT_WINDOW_WIDTH_PX,
            window_height_px: DEFAULT_WINDOW_HEIGHT_PX,
            pointer_position: (0.0, 0.0),
            needs_redraw: true,
            event_loop_proxy: None,
        })
    }

    pub fn editor_runtime_tab_count(&self) -> usize {
        self.editor_runtime.tab_count()
    }

    pub fn paths(&self) -> &NotoraPaths {
        &self.paths
    }

    pub fn state(&self) -> &NotoraState {
        &self.state
    }

    pub fn shell_layout(&self) -> ShellLayout {
        let dpi = self.editor_runtime.scale_factor() as f32;
        ShellLayout::compute(ShellLayoutInput {
            window_width_px: self.window_width_px,
            window_height_px: self.window_height_px,
            dpi,
            navigation_width_logical: self.state.layout.navigation_width_logical,
            card_list_width_logical: self.state.layout.card_list_width_logical,
        })
    }

    pub fn dispatch_action(&mut self, action: NotoraAction) {
        for effect in self.state.reduce(action) {
            let shell_effect = EffectExecutor::execute(self, effect);
            self.apply_shell_effect(shell_effect);
        }
    }

    pub fn update_editor_preedit(&mut self, text: String, cursor: Option<(usize, usize)>) -> bool {
        let context =
            events::editor_input_context(&self.state, self.shell_layout(), self.window_focused);
        self.editor_runtime.update_preedit(context, text, cursor)
    }

    pub fn set_event_loop_proxy(&mut self, event_loop_proxy: EventLoopProxy<ShellEvent>) {
        self.event_loop_proxy = Some(event_loop_proxy);
    }

    pub(crate) fn set_window_focused(&mut self, focused: bool) {
        self.window_focused = focused;
        self.editor_runtime.set_window_focus(focused);
    }

    pub(crate) fn set_window_size(&mut self, width: u32, height: u32) {
        self.window_width_px = width as f32;
        self.window_height_px = height as f32;
        self.state.layout.responsive_mode = self.shell_layout().responsive_mode;
    }

    pub(crate) fn editor_runtime_mut(&mut self) -> &mut EditorRuntime {
        &mut self.editor_runtime
    }

    pub(crate) fn take_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.needs_redraw) || self.editor_runtime.take_redraw_request()
    }

    pub(crate) fn request_window_redraw(&self) {
        if let Some(window) = self.editor_runtime.window() {
            window.request_redraw();
        }
    }

    pub(crate) fn runtime_accepts_keyboard_input(&self) -> bool {
        self.editor_runtime.keyboard_input_allowed(events::editor_input_context(
            &self.state,
            self.shell_layout(),
            self.window_focused,
        ))
    }

    pub(crate) fn runtime_accepts_pointer_input(&self, px: f32, py: f32) -> bool {
        self.editor_runtime.pointer_input_allowed(
            events::editor_input_context(&self.state, self.shell_layout(), self.window_focused),
            (px, py),
        )
    }

    pub(crate) fn begin_editor_text_selection(&mut self) -> bool {
        self.editor_runtime.begin_text_selection(events::editor_input_context(
            &self.state,
            self.shell_layout(),
            self.window_focused,
        ))
    }

    pub(crate) fn end_editor_pointer_capture(&mut self) {
        self.editor_runtime.end_pointer_capture();
    }

    pub(crate) fn set_pointer_position(&mut self, px: f32, py: f32) {
        self.pointer_position = (px, py);
    }

    pub(crate) fn pointer_position(&self) -> (f32, f32) {
        self.pointer_position
    }

    pub(crate) fn set_scale_factor(&mut self, scale_factor: f64) {
        self.editor_runtime.set_scale_factor(scale_factor);
        self.state.layout.responsive_mode = self.shell_layout().responsive_mode;
        self.needs_redraw = true;
    }

    pub(crate) fn resume(&mut self, event_loop: &ActiveEventLoop) -> Result<(), NotoraAppError> {
        if self.editor_runtime.window().is_some() {
            return Ok(());
        }
        let font_system = Arc::new(Mutex::new(shaping::font_cache::new_font_system_with_cache(
            &self.paths.config_directory.join("font-cache.bin"),
        )));
        self.editor_runtime.set_shared_font_system(Arc::clone(&font_system));
        self.editor_runtime
            .resume(
                event_loop,
                WindowAttributes::default().with_title("notora"),
                font_system,
                self.settings.font_size,
                &self.settings.font_family,
            )
            .map_err(NotoraAppError::Runtime)?;
        if let Some((width, height)) = self.editor_runtime.window().map(|window| {
            let size = window.inner_size();
            window.set_ime_allowed(true);
            (size.width, size.height)
        }) {
            self.set_window_size(width, height);
        }
        if let Some(event_loop_proxy) = self.event_loop_proxy.clone() {
            ProductHost::start_background_services(
                &mut self.product,
                ProductWakeHandle::new(event_loop_proxy),
            );
        }
        self.needs_redraw = true;
        Ok(())
    }

    pub(crate) fn resize_window(&mut self, width: u32, height: u32) {
        self.set_window_size(width, height);
        let _ = self.editor_runtime.resize_now(width, height);
        self.needs_redraw = true;
    }

    pub(crate) fn drain_product_events(&mut self) {
        let effect = ProductHost::drain_product_events(&mut self.product);
        self.apply_shell_effect(effect);
    }

    pub(crate) fn route_product_event(&mut self, event: &ui::Event) -> bool {
        let focus_target = self.state.layout.focus_target;
        let actions = self.shell.route_event(
            event,
            focus_target,
            &self.theme,
            self.editor_runtime.scale_factor() as f32,
        );
        if actions.is_empty() {
            return false;
        }
        for action in actions {
            self.dispatch_action(action);
        }
        true
    }

    pub(crate) fn render(&mut self) -> Result<(), RenderError> {
        self.needs_redraw = false;
        let layout = self.shell_layout();
        let model = NotoraRenderModel::from_state(&self.state);
        let mut render_resources = self.editor_runtime.take_render_resources();
        let mut frame = self.editor_runtime.begin_frame()?;
        self.shell.render(&mut frame, layout, &model)?;
        let mut vertices = Vec::new();
        frame.drain_into(
            ui::Screen::new(self.window_width_px, self.window_height_px),
            &mut render_resources,
            &mut vertices,
        );
        submit_shell_frame(&mut render_resources, &vertices, self.theme.editor.background);
        let _ = frame.present()?;
        self.editor_runtime.restore_render_resources(render_resources);
        self.editor_runtime.mark_frame_presented();
        Ok(())
    }

    pub(crate) fn shutdown(&mut self) {
        ProductHost::shutdown(&mut self.product);
        self.editor_runtime.shutdown();
    }

    fn apply_shell_effect(&mut self, effect: ShellEffect) {
        if effect.redraw {
            self.needs_redraw = true;
            self.editor_runtime.request_redraw();
        }
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

impl Default for NotoraApp {
    fn default() -> Self {
        Self::new()
    }
}

impl NotoraEffectService for NotoraApp {
    fn query_cards(&mut self, _query: CardQuery) {}

    fn request_note_creation(&mut self, _kind: DocumentKind, _target: NoteCreationTarget) {}

    fn prepare_document(&mut self, _identity: DocumentIdentity) {}

    fn persist_layout(&mut self) {}
}

fn build_editor_runtime(
    settings: &ui::Settings,
    theme: &ui::Theme,
    paths: &NotoraPaths,
) -> Result<EditorRuntime, NotoraAppError> {
    let mut plugin_registry = ui::plugin::PluginRegistry::new();
    plugin_registry.register(Box::new(EditorPluginFactory));
    let registered_plugin_ids = HashSet::from([PLUGIN_EDITOR]);
    let view_routes = ViewRouteTable::new(Vec::new(), &registered_plugin_ids)
        .map_err(EditorRuntimeError::InvalidRoute)
        .map_err(NotoraAppError::Runtime)?;
    EditorRuntime::new(EditorRuntimeConfig {
        plugin_registry,
        view_routes,
        initial_settings: settings.clone(),
        initial_theme: theme.clone(),
        snapshots_directory: paths.snapshots_directory.clone(),
    })
    .map_err(NotoraAppError::Runtime)
}

#[cfg(test)]
mod tests {
    use super::NotoraApp;
    use crate::action::NotoraAction;
    use crate::{FocusTarget, NotoraPaths, OverlayState};
    use notora_core::NavigationScope;

    fn app() -> NotoraApp {
        let directory = tempfile::tempdir().expect("test should create a temporary directory");
        let paths = NotoraPaths::from_config_directory(directory.keep().join("notora"))
            .expect("test should create isolated product paths");
        NotoraApp::with_paths(paths).expect("notora app should construct without a window")
    }

    #[test]
    fn document_ime_is_gated_by_product_focus_and_modal_state() {
        let mut app = app();
        assert!(!app.update_editor_preedit("拼".to_owned(), Some((0, 1))));

        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        assert!(app.update_editor_preedit("拼".to_owned(), Some((0, 1))));

        app.dispatch_action(NotoraAction::OpenSettings);
        assert_eq!(app.state().layout.overlay, OverlayState::Settings);
        assert!(!app.update_editor_preedit("音".to_owned(), Some((0, 1))));
    }

    #[test]
    fn search_ime_commit_is_consumed_before_it_can_reach_the_editor_runtime() {
        let mut app = app();
        app.dispatch_action(NotoraAction::FocusRequested(FocusTarget::Editor));
        assert!(app.update_editor_preedit("document".to_owned(), Some((0, 8))));
        app.render().expect("headless shell frame should render");

        assert!(app.route_product_event(&ui::Event::MouseDown {
            px: 24.0,
            py: 24.0,
            button: ui::core::widget::MouseButton::Left,
        }));
        assert_eq!(app.state().layout.focus_target, FocusTarget::NavigationSearch);
        assert!(app.route_product_event(&ui::Event::ImeCommit("路线图".to_owned())));

        assert_eq!(
            app.state().library.navigation_scope,
            NavigationScope::Search { query: "路线图".to_owned() }
        );
        assert_eq!(app.editor_runtime.preedit().0, "document");
    }
}
