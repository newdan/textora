//! 窗口、GPU、文本资源和 resize 时序的共享会话。

use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use winit::dpi::PhysicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

use crate::editor_runtime::{EditorRuntime, RenderResources};
use crate::frame_cache::FrameCache;
use crate::gpu::{self, GpuError};
use crate::render_state::{GpuState, TextState};

const RESIZE_THROTTLE: Duration = Duration::from_millis(16);
static EMPTY_FRAME_CACHE: LazyLock<FrameCache> = LazyLock::new(FrameCache::new);

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
    // None while a render scope owns the resources.
    resources: Option<RenderResources>,
    surface_size: Option<PhysicalSize<u32>>,
    scale_factor: f64,
    pending_resize: Option<PhysicalSize<u32>>,
    last_resize_handled: Instant,
    window_focused: bool,
    redraw_requested: bool,
    first_frame_presented: bool,
    last_render_time: Instant,
    render_frame_count: u32,
}

impl RenderSession {
    pub(crate) fn new() -> Self {
        Self {
            gpu_preparation: None,
            window: None,
            resources: Some(RenderResources::default()),
            surface_size: None,
            scale_factor: 1.0,
            pending_resize: None,
            last_resize_handled: Instant::now(),
            window_focused: true,
            redraw_requested: false,
            first_frame_presented: false,
            last_render_time: Instant::now(),
            render_frame_count: 0,
        }
    }

    pub(crate) fn start_gpu_preparation(&mut self) -> std::io::Result<()> {
        if self.gpu_preparation.is_some()
            || self.resources.as_ref().is_none_or(|resources| resources.gpu.is_some())
        {
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
        let resume_started_at = Instant::now();
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|error| GpuError::SurfaceCreation(error.to_string()))?,
        );
        let window_creation_elapsed = resume_started_at.elapsed();
        let size = window.inner_size();
        let gpu_wait_started_at = Instant::now();
        let preparation_result = self.gpu_preparation.take().map(|preparation| preparation.join());
        let gpu_wait_elapsed = gpu_wait_started_at.elapsed();
        let (prepared_gpu, preparation_status) = match preparation_result {
            Some(Ok(Ok(prepared))) => (Some(prepared), "ready"),
            Some(Ok(Err(error))) => {
                eprintln!("[startup:gpu_prepare] failed: {error}");
                (None, "failed")
            }
            Some(Err(_)) => (None, "panicked"),
            None => (None, "not_started"),
        };
        let scale_factor = window.scale_factor();
        let mut text_init_elapsed = Duration::ZERO;
        let gpu_context_started_at = Instant::now();
        let (gpu, text) = gpu::initialize_window_resources(
            window.clone(),
            size.width,
            size.height,
            prepared_gpu,
            |context| {
                let gpu = GpuState { ctx: context, size };
                let text_init_started_at = Instant::now();
                let text = gpu::with_gpu_error_scopes(&gpu.ctx.device, || {
                    TextState::init(
                        &gpu,
                        font_size * scale_factor as f32,
                        Arc::clone(&font_system),
                        font_family,
                    )
                });
                text_init_elapsed += text_init_started_at.elapsed();
                Ok((gpu, text?))
            },
        )?;
        let gpu_context_elapsed =
            gpu_context_started_at.elapsed().saturating_sub(text_init_elapsed);

        self.scale_factor = scale_factor;
        self.update_surface_size(size);
        self.window = Some(window);
        let resources = self.resources.get_or_insert_with(RenderResources::default);
        resources.gpu = Some(gpu);
        resources.text = Some(text);
        self.redraw_requested = true;
        self.first_frame_presented = false;
        eprintln!(
            "[startup:render_session] create_window={:.2}ms gpu_wait={:.2}ms gpu_prepare={preparation_status} gpu_context={:.2}ms text_init={:.2}ms total={:.2}ms",
            window_creation_elapsed.as_secs_f64() * 1_000.0,
            gpu_wait_elapsed.as_secs_f64() * 1_000.0,
            gpu_context_elapsed.as_secs_f64() * 1_000.0,
            text_init_elapsed.as_secs_f64() * 1_000.0,
            resume_started_at.elapsed().as_secs_f64() * 1_000.0,
        );
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
        if self.resources.is_none() && self.pending_resize.is_some() {
            return ResizeResult::Deferred;
        }

        let Some(size) = self.pending_resize.take() else {
            return ResizeResult::NotReady;
        };
        if size.width == 0 || size.height == 0 {
            return ResizeResult::NotReady;
        }

        let Some(gpu) = self.resources.as_mut().and_then(|resources| resources.gpu.as_mut()) else {
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
    }

    pub(crate) fn has_redraw_request(&self) -> bool {
        self.redraw_requested
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

    pub(crate) fn clear_frame_cluster_pool(&mut self) {
        if let Some(resources) = self.resources.as_mut() {
            resources.frame_cache.cluster_pool.clear();
        }
    }

    pub(crate) fn frame_cache(&self) -> &FrameCache {
        self.resources.as_ref().map_or(&EMPTY_FRAME_CACHE, |resources| &resources.frame_cache)
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

    pub(crate) fn shutdown(&mut self) {
        let resources = self.resources.get_or_insert_with(RenderResources::default);
        resources.text = None;
        resources.gpu = None;
        self.window = None;
        self.surface_size = None;
        self.pending_resize = None;
        self.redraw_requested = false;
        self.first_frame_presented = false;
    }
}

struct RenderResourcesScope<'a, Owner> {
    owner: &'a mut Owner,
    editor: fn(&mut Owner) -> &mut EditorRuntime,
    resources: RenderResources,
}

impl<Owner> Drop for RenderResourcesScope<'_, Owner> {
    fn drop(&mut self) {
        let session = &mut (self.editor)(self.owner).render_session;
        // A shutdown or resume during the scope replaces the resource slot.
        if session.resources.is_none() {
            session.resources = Some(std::mem::take(&mut self.resources));
        }
    }
}

pub(super) fn with_render_resources<Owner, Output>(
    owner: &mut Owner,
    editor: fn(&mut Owner) -> &mut EditorRuntime,
    render: impl FnOnce(&mut Owner, &mut RenderResources) -> Output,
) -> Output {
    let resources = editor(owner)
        .render_session
        .resources
        .take()
        .expect("render resources can only be borrowed by one scope at a time");
    let mut scope = RenderResourcesScope { owner, editor, resources };
    render(scope.owner, &mut scope.resources)
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
    fn render_scope_preserves_resources_and_surface_size_during_unwind() {
        const CACHED_DOCUMENT_OFFSET: usize = 42;

        let mut editor = super::super::tests::runtime_with_clean_tab();
        let surface_size = PhysicalSize::new(1600, 1200);
        editor.render_session.update_surface_size(surface_size);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            editor.with_render_resources(|editor, resources| {
                assert_eq!(editor.render_session.surface_size(), Some(surface_size));
                resources.frame_cache.first_line.doc_offset = CACHED_DOCUMENT_OFFSET;
                panic!("simulate an interrupted render");
            });
        }));

        assert!(result.is_err());
        assert_eq!(editor.render_session.surface_size(), Some(surface_size));
        assert_eq!(
            editor.render_session.frame_cache().first_line.doc_offset,
            CACHED_DOCUMENT_OFFSET
        );
    }

    #[test]
    fn render_scope_defers_resize_until_resources_return() {
        let mut editor = super::super::tests::runtime_with_clean_tab();
        let size = PhysicalSize::new(1600, 1200);

        editor.with_render_resources(|editor, _| {
            assert_eq!(editor.render_session.resize_now(size), ResizeResult::Deferred);
        });

        assert_eq!(editor.render_session.pending_resize, Some(size));
    }

    #[test]
    fn render_scope_does_not_restore_resources_after_shutdown() {
        const CACHED_DOCUMENT_OFFSET: usize = 42;
        let mut editor = super::super::tests::runtime_with_clean_tab();

        editor.with_render_resources(|editor, resources| {
            resources.frame_cache.first_line.doc_offset = CACHED_DOCUMENT_OFFSET;
            editor.shutdown();
        });

        assert_eq!(
            editor.render_session.frame_cache().first_line.doc_offset,
            FrameCache::default().first_line.doc_offset
        );
    }
}
