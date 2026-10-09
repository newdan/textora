use super::*;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::windows::EventLoopBuilderExtWindows;
use winit::window::{Window, WindowId};

#[derive(Default)]
struct WindowFallbackProbe {
    attempts: usize,
    outcome: Option<Result<(), GpuError>>,
}

impl WindowFallbackProbe {
    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), GpuError> {
        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_visible(false))
                .map_err(|error| GpuError::SurfaceCreation(error.to_string()))?,
        );
        let size = window.inner_size();
        let mut prepared = prepare_gpu_device_for_backend(wgpu::Backends::GL)?;
        prepared.remaining_backends.push(wgpu::Backends::DX12);
        initialize_window_resources(window, size.width, size.height, Some(prepared), |context| {
            self.attempts += 1;
            if self.attempts == 1 {
                return with_gpu_error_scopes(&context.device, || {
                    let _ = context.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("intentional window initialization failure"),
                        source: wgpu::ShaderSource::Wgsl("invalid shader for regression".into()),
                    });
                    Ok(())
                });
            }
            with_gpu_error_scopes(&context.device, || {
                let _ = render::GlyphRenderer::new(&context.device, context.format);
                Ok(())
            })
        })
    }
}

impl ApplicationHandler for WindowFallbackProbe {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.outcome = Some(self.initialize(event_loop));
        event_loop.exit();
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}

#[test]
#[ignore = "requires a Windows desktop and working OpenGL/DX12 drivers"]
fn attached_opengl_failure_recreates_dx12_resources_on_the_same_window() {
    let event_loop = EventLoop::builder()
        .with_any_thread(true)
        .build()
        .expect("Windows test event loop must initialize");
    let mut probe = WindowFallbackProbe::default();
    event_loop.run_app(&mut probe).expect("test event loop must complete");
    probe
        .outcome
        .expect("resumed must initialize the test window")
        .expect("DX12 must configure the former OpenGL window and create the text pipeline");
    assert_eq!(probe.attempts, 2, "text initialization must retry exactly once");
}
