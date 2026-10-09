use appkit_shell::ShellEvent;
use appkit_shell::editor_runtime::{EditorCursorBlinkPhase, EditorRuntime};
use winit::event_loop::EventLoopProxy;
use winit::window::CursorIcon;

#[cfg(target_os = "macos")]
#[path = "window_runtime/macos_titlebar.rs"]
mod macos_titlebar;

#[cfg(all(test, target_os = "macos"))]
#[test]
fn macos_frame_reserves_custom_title_bar_at_each_dpi() {
    for dpi in [1.0, 1.5, 2.0] {
        assert!(frame_state(None).title_height(dpi) > 0.0);
    }
}

#[cfg(target_os = "windows")]
#[path = "window_runtime/window_shape.rs"]
mod window_shape;

pub(super) fn frame_state(
    window: Option<&winit::window::Window>,
) -> ui::window_frame::WindowFrameState {
    use ui::window_frame::WindowFrameState;
    if cfg!(target_os = "macos") {
        return WindowFrameState::MacOs;
    }
    if !cfg!(target_os = "windows") {
        return WindowFrameState::Native;
    }
    if window.is_some_and(|window| window.is_maximized() || window.fullscreen().is_some()) {
        return WindowFrameState::Maximized;
    }
    WindowFrameState::Restored
}

pub(super) fn drag_window(
    window: &winit::window::Window,
) -> Result<(), winit::error::ExternalError> {
    #[cfg(target_os = "macos")]
    return macos_titlebar::drag_window(window);
    #[cfg(not(target_os = "macos"))]
    window.drag_window()
}

const DEFAULT_WINDOW_WIDTH_PX: f32 = 1_200.0;
const DEFAULT_WINDOW_HEIGHT_PX: f32 = 800.0;

/// UI 线程上的平台窗口瞬时状态与窄能力端口。
pub(super) struct WindowRuntime {
    #[cfg(target_os = "windows")]
    window_shape: window_shape::WindowShapeCache,
    focused: bool,
    width_px: f32,
    height_px: f32,
    pointer_position: (f32, f32),
    last_editor_cursor_visible: bool,
    redraw_requested: bool,
    event_loop_proxy: Option<EventLoopProxy<ShellEvent>>,
}

impl WindowRuntime {
    pub(super) fn new() -> Self {
        Self {
            #[cfg(target_os = "windows")]
            window_shape: window_shape::WindowShapeCache::default(),
            focused: true,
            width_px: DEFAULT_WINDOW_WIDTH_PX,
            height_px: DEFAULT_WINDOW_HEIGHT_PX,
            pointer_position: (0.0, 0.0),
            last_editor_cursor_visible: true,
            redraw_requested: true,
            event_loop_proxy: None,
        }
    }

    pub(super) fn set_event_loop_proxy(&mut self, proxy: EventLoopProxy<ShellEvent>) {
        self.event_loop_proxy = Some(proxy);
    }

    pub(super) fn synchronize_window_chrome(&mut self, editor_runtime: &EditorRuntime) {
        #[cfg(target_os = "macos")]
        if let Some(window) = editor_runtime.window() {
            macos_titlebar::align_traffic_lights(window);
            macos_titlebar::synchronize_pointer_dragging(window, self.pointer_position);
        }
        #[cfg(target_os = "windows")]
        if let Some(window) = editor_runtime.window()
            && let Err(error) = self.window_shape.synchronize(window)
        {
            eprintln!("[window] 无法同步圆角轮廓：{error}");
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let _ = editor_runtime;
    }

    pub(super) fn event_loop_proxy(&self) -> Option<EventLoopProxy<ShellEvent>> {
        self.event_loop_proxy.clone()
    }

    pub(super) fn set_focused(&mut self, focused: bool, editor_runtime: &mut EditorRuntime) {
        self.focused = focused;
        editor_runtime.set_window_focus(focused);
    }

    pub(super) fn is_focused(&self) -> bool {
        self.focused
    }

    pub(super) fn set_size(&mut self, width: u32, height: u32) {
        self.width_px = width as f32;
        self.height_px = height as f32;
    }

    pub(super) fn restore_size(&mut self, width_px: f32, height_px: f32) {
        self.width_px = width_px;
        self.height_px = height_px;
    }

    pub(super) fn size(&self) -> (f32, f32) {
        (self.width_px, self.height_px)
    }

    pub(super) fn set_pointer_position(
        &mut self,
        px: f32,
        py: f32,
        editor_runtime: &EditorRuntime,
    ) {
        self.pointer_position = (px, py);
        #[cfg(target_os = "macos")]
        if let Some(window) = editor_runtime.window() {
            macos_titlebar::synchronize_pointer_dragging(window, self.pointer_position);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = editor_runtime;
    }

    pub(super) fn pointer_position(&self) -> (f32, f32) {
        self.pointer_position
    }

    pub(super) fn request_redraw(&mut self, editor_runtime: &mut EditorRuntime) {
        self.redraw_requested = true;
        editor_runtime.request_redraw();
    }

    pub(super) fn schedule_redraw(&mut self) {
        self.redraw_requested = true;
    }

    pub(super) fn merge_redraw_request(&mut self, requested: bool) {
        self.redraw_requested |= requested;
    }

    #[cfg(test)]
    pub(super) fn redraw_is_requested(&self) -> bool {
        self.redraw_requested
    }

    pub(super) fn mark_frame_rendered(&mut self) {
        self.redraw_requested = false;
    }

    pub(super) fn take_redraw_request(
        &mut self,
        editor_runtime_requested: bool,
        text_cursor_blink_due: bool,
        editor_cursor_blink_phase: Option<EditorCursorBlinkPhase>,
    ) -> bool {
        let editor_cursor_blink_due = match editor_cursor_blink_phase {
            Some(phase) => {
                let changed = phase.visible != self.last_editor_cursor_visible;
                self.last_editor_cursor_visible = phase.visible;
                changed
            }
            None => {
                self.last_editor_cursor_visible = true;
                false
            }
        };
        std::mem::take(&mut self.redraw_requested)
            || editor_runtime_requested
            || text_cursor_blink_due
            || editor_cursor_blink_due
    }

    pub(super) fn request_window_redraw(&self, editor_runtime: &EditorRuntime) {
        if let Some(window) = editor_runtime.window() {
            window.request_redraw();
        }
    }

    pub(super) fn set_cursor(&self, editor_runtime: &EditorRuntime, cursor_icon: CursorIcon) {
        if let Some(window) = editor_runtime.window() {
            window.set_cursor(cursor_icon);
        }
    }

    pub(super) fn update_title(&self, editor_runtime: &EditorRuntime, title: &str) {
        if let Some(window) = editor_runtime.window() {
            window.set_title(title);
        }
    }
}
