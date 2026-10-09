use std::io;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, CreateRoundRectRgn, DeleteObject, SetWindowRgn,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

// GDI 的圆角矩形比传入的右/下界少覆盖一个像素，补偿后才保留最末描边。
const GDI_ROUND_RECT_EDGE_COMPENSATION: i32 = 1;

/// Windows 原生裁剪与 UI 描边使用同一个逻辑半径；操作只发生在 UI 线程。
#[derive(Default)]
pub(super) struct WindowShapeCache {
    last_attempt: Option<(isize, WindowShape)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowShape {
    Unclipped,
    Rounded { left: i32, top: i32, width: i32, height: i32, radius: i32 },
}

impl WindowShape {
    fn rounded(client: RECT, origin: POINT, outer: RECT, dpi: f64) -> Self {
        let width = (client.right - client.left).max(0);
        let height = (client.bottom - client.top).max(0);
        let radius = (f64::from(ui::rounded_surface_frame::SHELL_CORNER_RADIUS_LOGICAL) * dpi)
            .round() as i32;
        Self::Rounded {
            left: origin.x - outer.left,
            top: origin.y - outer.top,
            width,
            height,
            radius: radius.max(0).min(width / 2).min(height / 2),
        }
    }
}

impl WindowShapeCache {
    pub(super) fn synchronize(&mut self, window: &Window) -> io::Result<()> {
        if window.is_minimized() == Some(true) {
            return Ok(());
        }
        let handle = window.window_handle().map_err(io::Error::other)?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Ok(());
        };
        let hwnd = handle.hwnd.get() as HWND;
        let shape = if window.is_maximized() || window.fullscreen().is_some() {
            WindowShape::Unclipped
        } else {
            read_client_shape(hwnd, window.scale_factor())?
        };
        let signature = (handle.hwnd.get(), shape);
        if self.last_attempt == Some(signature) {
            return Ok(());
        }
        // SetWindowRgn 会触发位置消息；先记住请求，避免重入和失败后的逐帧重试。
        self.last_attempt = Some(signature);
        apply_shape(hwnd, shape)
    }
}

fn read_client_shape(hwnd: HWND, dpi: f64) -> io::Result<WindowShape> {
    let mut client = RECT::default();
    let mut origin = POINT::default();
    let mut outer = RECT::default();
    // SAFETY: hwnd 来自存活的 winit Window，输出指针指向当前栈上有效结构体。
    unsafe {
        if GetClientRect(hwnd, &mut client) == 0
            || ClientToScreen(hwnd, &mut origin) == 0
            || GetWindowRect(hwnd, &mut outer) == 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(WindowShape::rounded(client, origin, outer, dpi))
}

fn apply_shape(hwnd: HWND, shape: WindowShape) -> io::Result<()> {
    let region = match shape {
        WindowShape::Unclipped => null_mut(),
        WindowShape::Rounded { left, top, width, height, radius } => {
            if width == 0 || height == 0 {
                return Ok(());
            }
            // SAFETY: 创建独立 GDI 区域，无外部指针；椭圆尺寸是半径的两倍。
            let region = unsafe {
                CreateRoundRectRgn(
                    left,
                    top,
                    left + width + GDI_ROUND_RECT_EDGE_COMPENSATION,
                    top + height + GDI_ROUND_RECT_EDGE_COMPENSATION,
                    radius * 2,
                    radius * 2,
                )
            };
            if region.is_null() {
                return Err(io::Error::last_os_error());
            }
            region
        }
    };
    // SAFETY: hwnd 在调用期间有效；成功后区域所有权交给系统，不再 DeleteObject。
    if unsafe { SetWindowRgn(hwnd, region, 1) } != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if !region.is_null() {
        // SAFETY: 调用失败时区域所有权仍属于本函数，必须释放。
        unsafe { DeleteObject(region) };
    }
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Graphics::Gdi::{CreateRectRgn, GetWindowRgn, PtInRegion};
    use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_POPUP};

    struct TestWindow(HWND);

    impl Drop for TestWindow {
        fn drop(&mut self) {
            // SAFETY: 测试窗口在当前线程创建，唯一所有者在此释放。
            unsafe { DestroyWindow(self.0) };
        }
    }

    #[test]
    fn client_offsets_and_dpi_use_the_shared_radius() {
        for dpi in [1.0, 1.5, 2.0] {
            let shape = WindowShape::rounded(
                RECT { left: 0, top: 0, right: 1200, bottom: 800 },
                POINT { x: 105, y: 207 },
                RECT { left: 100, top: 200, right: 1305, bottom: 1007 },
                dpi,
            );
            assert_eq!(
                shape,
                WindowShape::Rounded {
                    left: 5,
                    top: 7,
                    width: 1200,
                    height: 800,
                    radius: (f64::from(ui::rounded_surface_frame::SHELL_CORNER_RADIUS_LOGICAL)
                        * dpi)
                        .round() as i32,
                }
            );
        }
    }

    #[test]
    fn native_region_rounds_corners_and_can_be_removed_for_maximizing() {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        // SAFETY: 系统预注册的 STATIC 类，字符串有效；创建不显示的测试专用窗口。
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                0,
                0,
                200,
                120,
                null_mut(),
                null_mut(),
                null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!hwnd.is_null(), "hidden test window must be created");
        let window = TestWindow(hwnd);
        for dpi in [1.0, 1.5, 2.0] {
            let shape =
                read_client_shape(window.0, dpi).expect("test window has valid client bounds");
            apply_shape(window.0, shape).expect("test window accepts a rounded region");
            // SAFETY: 独立查询区域，有效窗口；查询只复制形状，不转移区域所有权。
            unsafe {
                let region = CreateRectRgn(0, 0, 0, 0);
                assert!(!region.is_null(), "region query allocation must succeed");
                assert_ne!(GetWindowRgn(window.0, region), 0);
                assert_eq!(PtInRegion(region, 0, 0), 0);
                assert_ne!(PtInRegion(region, 100, 0), 0);
                assert_ne!(PtInRegion(region, 0, 60), 0);
                assert_ne!(PtInRegion(region, 199, 60), 0);
                assert_ne!(PtInRegion(region, 100, 119), 0);
                assert_eq!(PtInRegion(region, 199, 119), 0);
                DeleteObject(region);
            }
        }
        apply_shape(window.0, WindowShape::Unclipped)
            .expect("maximized window region can be cleared");
        // SAFETY: 同上，查询清除后的测试窗口并释放查询区域。
        unsafe {
            let region = CreateRectRgn(0, 0, 0, 0);
            assert!(!region.is_null(), "region query allocation must succeed");
            assert_eq!(GetWindowRgn(window.0, region), 0);
            DeleteObject(region);
        }
    }
}
