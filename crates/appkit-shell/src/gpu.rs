//! Shared GPU initialization logic.
//!
//! Used by both the windowed app and headless mode.

use std::sync::Arc;

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
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuError::NoAdapter => write!(f, "no GPU adapter available"),
            GpuError::DeviceCreation(msg) => write!(f, "device creation failed: {msg}"),
            GpuError::SurfaceCreation(msg) => write!(f, "surface creation failed: {msg}"),
            GpuError::NoSurfaceFormat => write!(f, "no suitable surface format"),
            GpuError::TextInit(msg) => write!(f, "text init failed: {msg}"),
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
    let instance = wgpu::Instance::default();
    let surface = instance
        .create_surface(window.clone())
        .map_err(|e| GpuError::SurfaceCreation(e.to_string()))?;
    let adapter = pollster::block_on(request_adapter(&instance, Some(&surface)))
        .ok_or(GpuError::NoAdapter)?;
    let (device, queue) = request_device(&adapter)?;

    configure_gpu_context(surface, &adapter, device, queue, width, height)
}

/// Request the adapter and device before a native window exists.
pub fn prepare_gpu_device() -> Result<PreparedGpuDevice, GpuError> {
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(request_adapter(&instance, None)).ok_or(GpuError::NoAdapter)?;
    let (device, queue) = request_device(&adapter)?;
    Ok(PreparedGpuDevice { instance, adapter, device, queue })
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
    let PreparedGpuDevice { instance, adapter, device, queue } = prepared;
    let surface = instance
        .create_surface(Arc::clone(&window))
        .map_err(|error| GpuError::SurfaceCreation(error.to_string()))?;
    if !adapter.is_surface_supported(&surface) {
        return create_gpu_context(window, width, height);
    }
    match configure_gpu_context(surface, &adapter, device, queue, width, height) {
        Err(GpuError::NoSurfaceFormat) => create_gpu_context(window, width, height),
        result => result,
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
    let surface_caps = surface.get_capabilities(adapter);
    // Prefer sRGB for correct color rendering (critical on macOS)
    let format = surface_caps
        .formats
        .iter()
        .find(|f| f.is_srgb())
        .or_else(|| surface_caps.formats.first())
        .copied()
        .ok_or(GpuError::NoSurfaceFormat)?;
    eprintln!("GPU surface format: {format:?}");

    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: width.max(1),
        height: height.max(1),
        present_mode: wgpu::PresentMode::AutoVsync,
        alpha_mode: surface_caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);

    let (msaa_tex, msaa_view) = create_msaa_texture(&device, &config);

    Ok(GpuContext { surface, device, queue, config, format, msaa_tex, msaa_view })
}

/// Request a GPU adapter with hardware-first fallback.
async fn request_adapter(
    instance: &wgpu::Instance,
    compatible_surface: Option<&wgpu::Surface<'_>>,
) -> Option<wgpu::Adapter> {
    let opts = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
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
    let instance = wgpu::Instance::default();

    let adapter = request_adapter(&instance, None).await.ok_or(GpuError::NoAdapter)?;

    let adapter_info = adapter.get_info();
    let info_string = format!("{} ({:?})", adapter_info.name, adapter_info.backend);

    let (_device, _queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("edit+ device"),
            required_features: wgpu::Features::DUAL_SOURCE_BLENDING,
            ..Default::default()
        })
        .await
        .map_err(|e| GpuError::DeviceCreation(e.to_string()))?;

    Ok(info_string)
}

#[cfg(test)]
mod tests {
    use super::GpuError;

    #[test]
    fn gpu_error_display_includes_the_underlying_cause() {
        let error = GpuError::DeviceCreation("adapter unavailable".to_owned());

        assert_eq!(error.to_string(), "device creation failed: adapter unavailable");
    }
}
