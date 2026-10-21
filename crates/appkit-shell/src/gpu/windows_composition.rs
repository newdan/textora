use std::sync::Arc;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::DirectComposition::{
    DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR, DCOMPOSITION_BORDER_MODE_SOFT,
    DCompositionCreateDevice2, IDCompositionClip, IDCompositionDesktopDevice,
    IDCompositionRectangleClip, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, WS_EX_NOREDIRECTIONBITMAP,
};
use windows::core::{IUnknown, Interface};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use super::GpuError;
use super::window_clip::WindowClip;

fn window_handle(window: &Window) -> Option<HWND> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };
    Some(HWND(handle.hwnd.get() as *mut _))
}

/// This style explicitly opts the window into an alpha-capable composition swapchain.
pub(super) fn is_required(window: &Window) -> bool {
    let Some(hwnd) = window_handle(window) else {
        return false;
    };
    // SAFETY: the native handle belongs to the live window borrowed for this call.
    let extended_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    extended_style & WS_EX_NOREDIRECTIONBITMAP.0 as isize != 0
}

/// Owns the visual tree; the surface must be dropped before this owner.
pub(super) struct WindowComposition {
    pub visual: IDCompositionVisual,
    clip: IDCompositionRectangleClip,
    _target: IDCompositionTarget,
    device: IDCompositionDesktopDevice,
    last_clip: Option<WindowClip>,
    // CompositionVisual surfaces retain the visual, but do not retain the HWND owner.
    _window: Arc<Window>,
}

impl WindowComposition {
    pub(super) fn new(window: Arc<Window>) -> Result<Self, GpuError> {
        let hwnd = window_handle(&window)
            .ok_or_else(|| GpuError::SurfaceCreation("missing Windows HWND".to_owned()))?;
        // SAFETY: all interfaces are live COM references; the Arc keeps hwnd alive.
        // A null rendering device is supported for DXGI swapchain-only composition.
        let composition = unsafe {
            let device: IDCompositionDesktopDevice =
                DCompositionCreateDevice2(None::<&IUnknown>).map_err(composition_error)?;
            let target = device.CreateTargetForHwnd(hwnd, true).map_err(composition_error)?;
            let visual: IDCompositionVisual = device
                .CreateVisual()
                .map_err(composition_error)?
                .cast()
                .map_err(composition_error)?;
            let clip = device.CreateRectangleClip().map_err(composition_error)?;
            visual.SetBorderMode(DCOMPOSITION_BORDER_MODE_SOFT).map_err(composition_error)?;
            visual
                .SetBitmapInterpolationMode(DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR)
                .map_err(composition_error)?;
            target.SetRoot(&visual).map_err(composition_error)?;
            Self { visual, clip, _target: target, device, last_clip: None, _window: window }
        };
        Ok(composition)
    }

    pub(super) fn synchronize(&mut self, geometry: WindowClip) -> Result<(), GpuError> {
        if self.last_clip != Some(geometry) {
            self.update_clip(geometry)?;
        }
        // wgpu assigns new swapchains to our visual during configure/resize, but
        // leaves Commit to the external visual owner, even when geometry is unchanged.
        // SAFETY: device is a live DirectComposition interface owned by this object.
        unsafe { self.device.Commit() }.map_err(composition_error)?;
        self.last_clip = Some(geometry);
        Ok(())
    }

    fn update_clip(&self, geometry: WindowClip) -> Result<(), GpuError> {
        // SAFETY: clip and visual are live interfaces from the same device; all
        // values are physical-pixel bounds clamped by WindowClip.
        unsafe {
            if geometry.radius == 0.0 {
                return self.visual.SetClip(None::<&IDCompositionClip>).map_err(composition_error);
            }
            self.clip.SetLeft2(0.0).map_err(composition_error)?;
            self.clip.SetTop2(0.0).map_err(composition_error)?;
            self.clip.SetRight2(geometry.width).map_err(composition_error)?;
            self.clip.SetBottom2(geometry.height).map_err(composition_error)?;
            self.clip.SetTopLeftRadiusX2(geometry.radius).map_err(composition_error)?;
            self.clip.SetTopLeftRadiusY2(geometry.radius).map_err(composition_error)?;
            self.clip.SetTopRightRadiusX2(geometry.radius).map_err(composition_error)?;
            self.clip.SetTopRightRadiusY2(geometry.radius).map_err(composition_error)?;
            self.clip.SetBottomLeftRadiusX2(geometry.radius).map_err(composition_error)?;
            self.clip.SetBottomLeftRadiusY2(geometry.radius).map_err(composition_error)?;
            self.clip.SetBottomRightRadiusX2(geometry.radius).map_err(composition_error)?;
            self.clip.SetBottomRightRadiusY2(geometry.radius).map_err(composition_error)?;
            self.visual.SetClip(&self.clip).map_err(composition_error)
        }
    }
}

fn composition_error(error: windows::core::Error) -> GpuError {
    GpuError::SurfaceCreation(format!("DirectComposition: {error}"))
}
