//! Shared GPU initialization logic.
//!
//! Used by both the windowed app and headless mode.

use std::sync::Arc;
use std::time::Instant;

mod startup;
#[cfg(all(test, target_os = "windows"))]
mod windows_tests;
pub(crate) use startup::with_gpu_error_scopes;
use startup::{backend_candidates, try_backends};

/// Errors that can occur during GPU initialization.
#[derive(Debug)]
pub enum GpuError {
    /// No GPU adapter found (neither hardware nor software).
    NoAdapter,
    /// Failed to create a logical device.
    DeviceCreation(String),
    /// Failed to create a window surface.
    SurfaceCreation(String),
    /// No suitable surface format found.
    NoSurfaceFormat,
    /// Text rendering subsystem initialization failed.
    TextInit(String),
    /// GPU resources or surface configuration could not be initialized.
    ResourceCreation(String),
    /// Every requested backend failed; includes each backend's cause.
    BackendInitialization(String),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuError::NoAdapter => write!(f, "no GPU adapter available"),
            GpuError::DeviceCreation(msg) => write!(f, "device creation failed: {msg}"),
            GpuError::SurfaceCreation(msg) => write!(f, "surface creation failed: {msg}"),
            GpuError::NoSurfaceFormat => write!(f, "no suitable surface format"),
            GpuError::TextInit(msg) => write!(f, "text init failed: {msg}"),
            GpuError::ResourceCreation(message) => {
                write!(f, "GPU resource creation failed: {message}")
            }
            GpuError::BackendInitialization(message) => write!(f, "GPU backends failed: {message}"),
        }
    }
}

impl std::error::Error for GpuError {}

/// A configured GPU context (device, queue, surface config).
pub struct GpuContext {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub format: wgpu::TextureFormat,
    /// MSAA multisample target (recreated on resize).
    pub msaa_tex: wgpu::Texture,
    pub msaa_view: wgpu::TextureView,
}

/// A surface-independent adapter and device prepared before a window is available.
pub struct PreparedGpuDevice {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    requested_backends: wgpu::Backends,
    remaining_backends: Vec<wgpu::Backends>,
}

fn startup_instance(backends: wgpu::Backends) -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backends;
    wgpu::Instance::new(descriptor)
}

fn startup_power_preference() -> wgpu::PowerPreference {
    wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::HighPerformance)
}

/// Create a multisampled texture for MSAA resolve.
fn create_msaa_texture(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("msaa target"),
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 4,
        dimension: wgpu::TextureDimension::D2,
        format: config.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

impl GpuContext {
    /// Recreate the MSAA texture after surface resize.
    pub fn recreate_msaa(&mut self) {
        let (tex, view) = create_msaa_texture(&self.device, &self.config);
        self.msaa_tex = tex;
        self.msaa_view = view;
    }
}

/// Create a GPU context with a window surface.
///
/// Tries a hardware adapter first, then falls back to software.
pub fn create_gpu_context(
    window: Arc<winit::window::Window>,
    width: u32,
    height: u32,
) -> Result<GpuContext, GpuError> {
    initialize_window_resources(window, width, height, None, Ok)
}

fn create_gpu_context_for_backend(
    window: Arc<winit::window::Window>,
    width: u32,
    height: u32,
    requested_backends: wgpu::Backends,
) -> Result<GpuContext, GpuError> {
    let initialization_started_at = Instant::now();
    let instance = startup_instance(requested_backends);
    let instance_elapsed = initialization_started_at.elapsed();
    let surface_started_at = Instant::now();
    let surface = instance
        .create_surface(window.clone())
        .map_err(|e| GpuError::SurfaceCreation(e.to_string()))?;
    let surface_elapsed = surface_started_at.elapsed();
    let adapter_started_at = Instant::now();
    let adapter = pollster::block_on(request_adapter(&instance, Some(&surface)))
        .ok_or(GpuError::NoAdapter)?;
    let adapter_elapsed = adapter_started_at.elapsed();
    let device_started_at = Instant::now();
    let (device, queue) = request_device(&adapter)?;
    let device_elapsed = device_started_at.elapsed();

    let context = configure_gpu_context(surface, &adapter, device, queue, width, height)?;
    let adapter_info = adapter.get_info();
    eprintln!(
        "[startup:gpu_sync] requested_backends={requested_backends:?} power_preference={:?} adapter={} backend={:?} instance={:.2}ms surface={:.2}ms adapter_request={:.2}ms device_request={:.2}ms total={:.2}ms",
        startup_power_preference(),
        adapter_info.name,
        adapter_info.backend,
        instance_elapsed.as_secs_f64() * 1_000.0,
        surface_elapsed.as_secs_f64() * 1_000.0,
        adapter_elapsed.as_secs_f64() * 1_000.0,
        device_elapsed.as_secs_f64() * 1_000.0,
        initialization_started_at.elapsed().as_secs_f64() * 1_000.0,
    );
    Ok(context)
}

/// Request the adapter and device before a native window exists.
pub fn prepare_gpu_device() -> Result<PreparedGpuDevice, GpuError> {
    let mut candidates = backend_candidates(wgpu::Backends::from_env()).into_iter();
    let mut prepared = try_backends(&mut candidates, prepare_gpu_device_for_backend)?;
    prepared.remaining_backends.extend(candidates);
    Ok(prepared)
}

fn prepare_gpu_device_for_backend(
    requested_backends: wgpu::Backends,
) -> Result<PreparedGpuDevice, GpuError> {
    let preparation_started_at = Instant::now();
    let instance = startup_instance(requested_backends);
    let instance_elapsed = preparation_started_at.elapsed();
    let adapter_started_at = Instant::now();
    let adapter =
        pollster::block_on(request_adapter(&instance, None)).ok_or(GpuError::NoAdapter)?;
    let adapter_elapsed = adapter_started_at.elapsed();
    let device_started_at = Instant::now();
    let (device, queue) = request_device(&adapter)?;
    let adapter_info = adapter.get_info();
    eprintln!(
        "[startup:gpu_prepare] requested_backends={requested_backends:?} power_preference={:?} adapter={} backend={:?} instance={:.2}ms adapter_request={:.2}ms device_request={:.2}ms total={:.2}ms",
        startup_power_preference(),
        adapter_info.name,
        adapter_info.backend,
        instance_elapsed.as_secs_f64() * 1_000.0,
        adapter_elapsed.as_secs_f64() * 1_000.0,
        device_started_at.elapsed().as_secs_f64() * 1_000.0,
        preparation_started_at.elapsed().as_secs_f64() * 1_000.0,
    );
    Ok(PreparedGpuDevice {
        instance,
        adapter,
        device,
        queue,
        requested_backends,
        remaining_backends: Vec::new(),
    })
}

/// Attach a prepared adapter and device to a newly created window surface.
///
/// A surface-less adapter may be incompatible on some platforms. In that case,
/// retry through the existing surface-aware synchronous path.
pub fn create_gpu_context_from_prepared_device(
    window: Arc<winit::window::Window>,
    width: u32,
    height: u32,
    prepared: PreparedGpuDevice,
) -> Result<GpuContext, GpuError> {
    initialize_window_resources(window, width, height, Some(prepared), Ok)
}

/// Treat surface attachment and dependent render resources as one backend attempt.
pub(crate) fn initialize_window_resources<T>(
    window: Arc<winit::window::Window>,
    width: u32,
    height: u32,
    mut prepared: Option<PreparedGpuDevice>,
    mut initialize: impl FnMut(GpuContext) -> Result<T, GpuError>,
) -> Result<T, GpuError> {
    let candidates = prepared
        .as_ref()
        .map(|prepared| {
            std::iter::once(prepared.requested_backends)
                .chain(prepared.remaining_backends.iter().copied())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| backend_candidates(wgpu::Backends::from_env()));
    try_backends(&mut candidates.into_iter(), |backends| {
        let context = match prepared.take() {
            Some(prepared) => attach_prepared_device(window.clone(), width, height, prepared),
            None => create_gpu_context_for_backend(window.clone(), width, height, backends),
        }?;
        initialize(context)
    })
}

fn attach_prepared_device(
    window: Arc<winit::window::Window>,
    width: u32,
    height: u32,
    prepared: PreparedGpuDevice,
) -> Result<GpuContext, GpuError> {
    let attachment_started_at = Instant::now();
    let PreparedGpuDevice { instance, adapter, device, queue, requested_backends, .. } = prepared;
    let surface = instance
        .create_surface(Arc::clone(&window))
        .map_err(|error| GpuError::SurfaceCreation(error.to_string()))?;
    let surface_elapsed = attachment_started_at.elapsed();
    if !adapter.is_surface_supported(&surface) {
        eprintln!("[startup:gpu_prepared] fallback=unsupported_surface");
        drop((surface, adapter, device, queue, instance));
        return create_gpu_context_for_backend(window, width, height, requested_backends);
    }
    let configured_context = configure_gpu_context(surface, &adapter, device, queue, width, height);
    let attachment_elapsed = attachment_started_at.elapsed();
    match configured_context {
        Err(GpuError::NoSurfaceFormat) => {
            eprintln!("[startup:gpu_prepared] fallback=no_surface_format");
            drop((adapter, instance));
            create_gpu_context_for_backend(window, width, height, requested_backends)
        }
        result => {
            eprintln!(
                "[startup:gpu_prepared] surface={:.2}ms configure={:.2}ms total={:.2}ms",
                surface_elapsed.as_secs_f64() * 1_000.0,
                attachment_elapsed.saturating_sub(surface_elapsed).as_secs_f64() * 1_000.0,
                attachment_elapsed.as_secs_f64() * 1_000.0,
            );
            result
        }
    }
}

fn request_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), GpuError> {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("edit+ device"),
        required_features: wgpu::Features::DUAL_SOURCE_BLENDING,
        ..Default::default()
    }))
    .map_err(|error| GpuError::DeviceCreation(error.to_string()))
}

fn configure_gpu_context(
    surface: wgpu::Surface<'static>,
    adapter: &wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,
) -> Result<GpuContext, GpuError> {
    let scope_device = device.clone();
    with_gpu_error_scopes(&scope_device, || {
        configure_gpu_context_resources(surface, adapter, device, queue, width, height)
    })
}

fn configure_gpu_context_resources(
    surface: wgpu::Surface<'static>,
    adapter: &wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    width: u32,
    height: u32,
) -> Result<GpuContext, GpuError> {
    let configuration_started_at = Instant::now();
    let surface_caps = surface.get_capabilities(adapter);
    // Prefer sRGB for correct color rendering (critical on macOS)
    let format = surface_caps
        .formats
        .iter()
        .find(|f| f.is_srgb())
        .or_else(|| surface_caps.formats.first())
        .copied()
        .ok_or(GpuError::NoSurfaceFormat)?;
    let capabilities_elapsed = configuration_started_at.elapsed();

    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: width.max(1),
        height: height.max(1),
        present_mode: wgpu::PresentMode::AutoVsync,
        alpha_mode: surface_caps.alpha_modes.first().copied().ok_or(GpuError::NoSurfaceFormat)?,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    let surface_configuration_started_at = Instant::now();
    surface.configure(&device, &config);
    let surface_configuration_elapsed = surface_configuration_started_at.elapsed();

    let msaa_started_at = Instant::now();
    let (msaa_tex, msaa_view) = create_msaa_texture(&device, &config);
    let msaa_elapsed = msaa_started_at.elapsed();
    let configuration_elapsed = configuration_started_at.elapsed();
    eprintln!("GPU surface format: {format:?}");
    eprintln!(
        "[startup:gpu_configure] capabilities={:.2}ms surface_configure={:.2}ms msaa_texture={:.2}ms total={:.2}ms",
        capabilities_elapsed.as_secs_f64() * 1_000.0,
        surface_configuration_elapsed.as_secs_f64() * 1_000.0,
        msaa_elapsed.as_secs_f64() * 1_000.0,
        configuration_elapsed.as_secs_f64() * 1_000.0,
    );

    Ok(GpuContext { surface, device, queue, config, format, msaa_tex, msaa_view })
}

/// Request a GPU adapter with hardware-first fallback.
async fn request_adapter(
    instance: &wgpu::Instance,
    compatible_surface: Option<&wgpu::Surface<'_>>,
) -> Option<wgpu::Adapter> {
    let opts = wgpu::RequestAdapterOptions {
        power_preference: startup_power_preference(),
        compatible_surface,
        force_fallback_adapter: false,
    };

    if let Ok(adapter) = instance.request_adapter(&opts).await {
        return Some(adapter);
    }

    // Fallback to software adapter
    let fallback_opts = wgpu::RequestAdapterOptions { force_fallback_adapter: true, ..opts };
    instance.request_adapter(&fallback_opts).await.ok()
}

/// Headless GPU initialization (no surface).
///
/// Returns the adapter info string on success.
pub async fn headless_init() -> Result<String, GpuError> {
    let prepared = prepare_gpu_device()?;
    let adapter_info = prepared.adapter.get_info();
    Ok(format!("{} ({:?})", adapter_info.name, adapter_info.backend))
}

#[cfg(test)]
mod tests {
    use super::GpuError;

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_build_includes_opengl_and_dx12() {
        let enabled = wgpu::Instance::enabled_backend_features();
        assert!(enabled.contains(wgpu::Backends::GL));
        assert!(enabled.contains(wgpu::Backends::DX12));
    }

    #[test]
    fn backend_override_is_read_in_a_separate_process() {
        let executable = std::env::current_exe().expect("test executable path must be available");
        let outcome = std::process::Command::new(executable)
            .args(["--ignored", "--exact", "gpu::tests::backend_override_child"])
            .env("WGPU_BACKEND", "gl")
            .output()
            .expect("backend override test subprocess must start");
        assert!(outcome.status.success(), "{}", String::from_utf8_lossy(&outcome.stdout));
    }

    #[test]
    #[ignore = "run by backend_override_is_read_in_a_separate_process with its own environment"]
    fn backend_override_child() {
        assert_eq!(wgpu::Backends::from_env(), Some(wgpu::Backends::GL));
    }

    #[test]
    #[cfg(target_os = "windows")]
    #[ignore = "requires working OpenGL and DX12 drivers; run explicitly on Windows"]
    fn native_shader_failure_falls_back_to_dx12() {
        let mut captured_shader_failure = false;
        let chosen = super::try_backends(
            &mut [wgpu::Backends::GL, wgpu::Backends::DX12].into_iter(),
            |backends| {
                let prepared = super::prepare_gpu_device_for_backend(backends)?;
                if backends == wgpu::Backends::GL {
                    let compilation = super::with_gpu_error_scopes(&prepared.device, || {
                        let _ =
                            prepared.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                                label: Some("intentional startup failure regression"),
                                source: wgpu::ShaderSource::Wgsl(
                                    "invalid shader for regression".into(),
                                ),
                            });
                        Ok(())
                    });
                    captured_shader_failure =
                        matches!(compilation, Err(GpuError::ResourceCreation(_)));
                    compilation?;
                }
                Ok(prepared.adapter.get_info().backend)
            },
        )
        .expect("DX12 should succeed after the intentionally invalid OpenGL shader");
        assert!(
            captured_shader_failure,
            "the test must exercise an actual OpenGL validation error"
        );
        assert_eq!(chosen, wgpu::Backend::Dx12);
    }

    #[test]
    fn gpu_error_display_includes_the_underlying_cause() {
        let error = GpuError::DeviceCreation("adapter unavailable".to_owned());

        assert_eq!(error.to_string(), "device creation failed: adapter unavailable");
    }
}
