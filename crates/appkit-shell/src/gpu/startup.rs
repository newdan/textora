use super::GpuError;

pub(super) fn backend_candidates(override_backends: Option<wgpu::Backends>) -> Vec<wgpu::Backends> {
    if let Some(backends) = override_backends {
        return vec![backends];
    }
    if cfg!(target_os = "windows") {
        return vec![wgpu::Backends::GL, wgpu::Backends::DX12];
    }
    vec![wgpu::InstanceDescriptor::new_without_display_handle().backends]
}

/// Each backend is initialized lazily; successful preparation leaves later candidates unconsumed.
pub(super) fn try_backends<T>(
    candidates: &mut impl Iterator<Item = wgpu::Backends>,
    mut initialize: impl FnMut(wgpu::Backends) -> Result<T, GpuError>,
) -> Result<T, GpuError> {
    let mut failures = Vec::new();
    for backends in candidates {
        match initialize(backends) {
            Ok(initialized) => return Ok(initialized),
            Err(error) => {
                eprintln!("[startup:gpu_fallback] backends={backends:?} error={error}");
                failures.push((backends, error));
            }
        }
    }
    if failures.iter().all(|(_, error)| matches!(error, GpuError::NoAdapter)) {
        return Err(GpuError::NoAdapter);
    }
    let causes = failures
        .into_iter()
        .map(|(backends, error)| format!("{backends:?}: {error}"))
        .collect::<Vec<_>>()
        .join("; ");
    Err(GpuError::BackendInitialization(causes))
}

/// Convert recoverable wgpu initialization errors into a failed backend attempt.
pub(crate) fn with_gpu_error_scopes<T>(
    device: &wgpu::Device,
    initialize: impl FnOnce() -> Result<T, GpuError>,
) -> Result<T, GpuError> {
    let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let initialized = initialize();
    let mut first_error = None;
    for scope in [validation, internal, out_of_memory] {
        if let Some(error) = pollster::block_on(scope.pop()) {
            first_error.get_or_insert(error);
        }
    }
    if let Some(error) = first_error {
        return Err(GpuError::ResourceCreation(error.to_string()));
    }
    initialized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_default_attempts_opengl_before_dx12() {
        assert_eq!(backend_candidates(None), [wgpu::Backends::GL, wgpu::Backends::DX12]);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn other_platforms_keep_the_existing_backend_set() {
        assert_eq!(
            backend_candidates(None),
            [wgpu::InstanceDescriptor::new_without_display_handle().backends]
        );
    }

    #[test]
    fn successful_opengl_never_initializes_dx12() {
        let mut candidates = [wgpu::Backends::GL, wgpu::Backends::DX12].into_iter();
        let mut attempted = Vec::new();
        let chosen = try_backends(&mut candidates, |backend| {
            attempted.push(backend);
            Ok(backend)
        })
        .expect("first backend succeeds");
        assert_eq!(chosen, wgpu::Backends::GL);
        assert_eq!(attempted, [wgpu::Backends::GL]);
        assert_eq!(candidates.collect::<Vec<_>>(), [wgpu::Backends::DX12]);
    }

    #[test]
    fn initialization_failures_advance_to_dx12_once() {
        let failures = [
            GpuError::NoAdapter,
            GpuError::DeviceCreation("dual-source blending unavailable".to_owned()),
            GpuError::SurfaceCreation("window surface unavailable".to_owned()),
            GpuError::NoSurfaceFormat,
            GpuError::ResourceCreation("surface configuration failed".to_owned()),
            GpuError::ResourceCreation("shader compilation failed".to_owned()),
            GpuError::TextInit("text resources unavailable".to_owned()),
        ];
        for failure in failures {
            let mut failure = Some(failure);
            let mut attempted = Vec::new();
            let chosen = try_backends(
                &mut [wgpu::Backends::GL, wgpu::Backends::DX12].into_iter(),
                |backend| {
                    attempted.push(backend);
                    match failure.take() {
                        Some(error) => Err(error),
                        None => Ok(backend),
                    }
                },
            )
            .expect("DX12 succeeds after OpenGL fails");
            assert_eq!(chosen, wgpu::Backends::DX12);
            assert_eq!(attempted, [wgpu::Backends::GL, wgpu::Backends::DX12]);
        }
    }

    #[test]
    fn exhausted_backends_report_both_causes_without_retrying() {
        let mut attempted = Vec::new();
        let failure = try_backends::<()>(
            &mut [wgpu::Backends::GL, wgpu::Backends::DX12].into_iter(),
            |backend| {
                attempted.push(backend);
                Err(GpuError::DeviceCreation("driver unavailable".to_owned()))
            },
        )
        .expect_err("neither backend is available");
        assert_eq!(attempted.len(), 2);
        assert!(failure.to_string().contains("GL"));
        assert!(failure.to_string().contains("DX12"));
    }

    #[test]
    fn missing_adapters_preserve_the_headless_skip_error() {
        let failure =
            try_backends::<()>(&mut [wgpu::Backends::GL, wgpu::Backends::DX12].into_iter(), |_| {
                Err(GpuError::NoAdapter)
            });
        assert!(matches!(failure, Err(GpuError::NoAdapter)));
    }

    #[test]
    fn explicit_override_is_not_expanded_with_other_backends() {
        for selected in [wgpu::Backends::GL, wgpu::Backends::VULKAN, wgpu::Backends::empty()] {
            let mut candidates = backend_candidates(Some(selected)).into_iter();
            let mut attempted = Vec::new();
            let failure = try_backends::<()>(&mut candidates, |backend| {
                attempted.push(backend);
                Err(GpuError::NoAdapter)
            });
            assert!(failure.is_err());
            assert_eq!(attempted, [selected]);
        }
    }
}
