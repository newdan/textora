//! 窗口、GPU、文本资源和 resize 时序的共享会话。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use winit::dpi::PhysicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

use crate::editor_runtime::RenderResources;
use crate::frame_cache::FrameCache;
use crate::gpu::{self, GpuError};
use crate::render_state::{GpuState, TextState};

const RESIZE_THROTTLE: Duration = Duration::from_millis(16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResizeResult {
    NotReady,
    Deferred,
    Applied { width_changed: bool },
}

/// Runtime 持有的窗口和绘制资源；产品只通过语义方法驱动它。
pub(crate) struct RenderSession {
    gpu_preparation: Option<
        std::thread::JoinHandle<Result<crate::gpu::PreparedGpuDevice, crate::gpu::GpuError>>,
    >,
    window: Option<Arc<Window>>,
    gpu: Option<GpuState>,
    text: Option<TextState>,
    surface_size: Option<PhysicalSize<u32>>,
    frame_cache: FrameCache,
    scale_factor: f64,
    pending_resize: Option<PhysicalSize<u32>>,
    last_resize_handled: Instant,
    window_focused: bool,
    redraw_requested: bool,
    first_frame_presented: bool,
    last_render_time: Instant,
    last_result_time: Instant,
    render_frame_count: u32,
}

impl RenderSession {
    pub(crate) fn new() -> Self {
        Self {
            gpu_preparation: None,
            window: None,
            gpu: None,
            text: None,
            surface_size: None,
            frame_cache: FrameCache::new(),
            scale_factor: 1.0,
            pending_resize: None,
            last_resize_handled: Instant::now(),
            window_focused: true,
            redraw_requested: false,
            first_frame_presented: false,
            last_render_time: Instant::now(),
            last_result_time: Instant::now(),
            render_frame_count: 0,
        }
    }

    pub(crate) fn start_gpu_preparation(&mut self) -> std::io::Result<()> {
        if self.gpu_preparation.is_some() || self.gpu.is_some() {
            return Ok(());
        }
        self.gpu_preparation = Some(
            std::thread::Builder::new()
                .name("textora-gpu-preparation".to_owned())
                .spawn(gpu::prepare_gpu_device)?,
        );
        Ok(())
    }

    pub(crate) fn resume(
        &mut self,
        event_loop: &ActiveEventLoop,
        attributes: WindowAttributes,
        font_system: Arc<Mutex<shaping::FontSystem>>,
        font_size: f32,
        font_family: &str,
    ) -> Result<(), GpuError> {
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|error| GpuError::SurfaceCreation(error.to_string()))?,
        );
        let size = window.inner_size();
        let prepared_gpu = self
            .gpu_preparation
            .take()
            .and_then(|preparation| preparation.join().ok())
            .and_then(Result::ok);
        let gpu_context = match prepared_gpu {
            Some(prepared) => gpu::create_gpu_context_from_prepared_device(
                window.clone(),
                size.width,
                size.height,
                prepared,
            )?,
            None => gpu::create_gpu_context(window.clone(), size.width, size.height)?,
        };
        let gpu = GpuState { ctx: gpu_context, size };
        let scale_factor = window.scale_factor();
        let text =
            TextState::init(&gpu, font_size * scale_factor as f32, font_system, font_family)?;

        self.scale_factor = scale_factor;
        self.update_surface_size(size);
        self.window = Some(window);
        self.gpu = Some(gpu);
        self.text = Some(text);
        self.redraw_requested = true;
        self.first_frame_presented = false;
        Ok(())
    }

    pub(crate) fn request_resize(&mut self, size: PhysicalSize<u32>) -> ResizeResult {
        self.pending_resize = Some(size);
        if Instant::now().duration_since(self.last_resize_handled) < RESIZE_THROTTLE {
            return ResizeResult::Deferred;
        }
        self.flush_pending_resize()
    }

    pub(crate) fn resize_now(&mut self, size: PhysicalSize<u32>) -> ResizeResult {
        self.pending_resize = Some(size);
        self.last_resize_handled =
            Instant::now().checked_sub(RESIZE_THROTTLE).unwrap_or_else(Instant::now);
        self.flush_pending_resize()
    }

    pub(crate) fn flush_pending_resize(&mut self) -> ResizeResult {
        let Some(size) = self.pending_resize.take() else {
            return ResizeResult::NotReady;
        };
        if size.width == 0 || size.height == 0 {
            return ResizeResult::NotReady;
        }

        let Some(gpu) = self.gpu.as_mut() else {
            return ResizeResult::NotReady;
        };
        let width_changed = gpu.ctx.config.width != size.width;
        gpu.size = size;
        gpu.ctx.config.width = size.width;
        gpu.ctx.config.height = size.height;
        gpu.ctx.surface.configure(&gpu.ctx.device, &gpu.ctx.config);
        gpu.ctx.recreate_msaa();
        self.update_surface_size(size);
        self.last_resize_handled = Instant::now();
        self.redraw_requested = true;
        ResizeResult::Applied { width_changed }
    }

    pub(crate) fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;
        self.redraw_requested = true;
    }

    pub(crate) fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    pub(crate) fn set_window_focused(&mut self, focused: bool) {
        self.window_focused = focused;
    }

    pub(crate) fn window_focused(&self) -> bool {
        self.window_focused
    }

    pub(crate) fn request_redraw(&mut self) {
        self.redraw_requested = true;
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    pub(crate) fn take_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.redraw_requested)
    }

    pub(crate) fn mark_first_frame_presented(&mut self) {
        self.first_frame_presented = true;
    }

    pub(crate) fn first_frame_presented(&self) -> bool {
        self.first_frame_presented
    }

    pub(crate) fn window(&self) -> Option<&Arc<Window>> {
        self.window.as_ref()
    }

    pub(crate) fn surface_size(&self) -> Option<PhysicalSize<u32>> {
        self.surface_size
    }

    fn update_surface_size(&mut self, size: PhysicalSize<u32>) {
        self.surface_size = Some(size);
    }

    pub(crate) fn take_render_resources(&mut self) -> RenderResources {
        RenderResources {
            text: self.text.take(),
            gpu: self.gpu.take(),
            frame_cache: std::mem::take(&mut self.frame_cache),
        }
    }

    pub(crate) fn restore_render_resources(&mut self, resources: RenderResources) {
        self.text = resources.text;
        self.gpu = resources.gpu;
        self.frame_cache = resources.frame_cache;
    }

    pub(crate) fn clear_frame_cluster_pool(&mut self) {
        self.frame_cache.cluster_pool.clear();
    }

    pub(crate) fn frame_cache_snapshot(&self) -> FrameCache {
        self.frame_cache.clone()
    }

    pub(crate) fn note_render_started(&mut self, started_at: Instant) -> u128 {
        let interval = started_at.duration_since(self.last_render_time).as_micros();
        self.last_render_time = started_at;
        self.render_frame_count = self.render_frame_count.wrapping_add(1);
        interval
    }

    pub(crate) fn render_frame_count(&self) -> u32 {
        self.render_frame_count
    }

    pub(crate) fn note_result_arrived(&mut self, arrived_at: Instant) -> u128 {
        let interval = arrived_at.duration_since(self.last_result_time).as_micros();
        self.last_result_time = arrived_at;
        interval
    }

    pub(crate) fn shutdown(&mut self) {
        self.text = None;
        self.gpu = None;
        self.window = None;
        self.surface_size = None;
        self.pending_resize = None;
        self.redraw_requested = false;
        self.first_frame_presented = false;
    }
}

impl Default for RenderSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_without_window_is_safe_and_deferred() {
        let mut session = RenderSession::new();
        assert_eq!(session.request_resize(PhysicalSize::new(640, 480)), ResizeResult::Deferred);
        assert_eq!(session.flush_pending_resize(), ResizeResult::NotReady);
    }

    #[test]
    fn focus_and_redraw_state_are_product_neutral() {
        let mut session = RenderSession::new();
        assert!(session.window_focused());
        session.set_window_focused(false);
        session.request_redraw();
        assert!(!session.window_focused());
        assert!(session.take_redraw_request());
        assert!(!session.take_redraw_request());
    }

    #[test]
    fn shutdown_resets_first_frame_state() {
        let mut session = RenderSession::new();
        session.mark_first_frame_presented();
        assert!(session.first_frame_presented());
        session.shutdown();
        assert!(!session.first_frame_presented());
    }

    #[test]
    fn surface_size_survives_temporary_render_resource_take() {
        let mut session = RenderSession::new();
        let expected_size = PhysicalSize::new(1600, 1200);
        session.update_surface_size(expected_size);

        let _resources = session.take_render_resources();

        assert_eq!(session.surface_size(), Some(expected_size));
    }
}
