use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use appkit_shell::accessibility_adapter::PlatformAccessibilityAdapter;
use appkit_shell::editor_runtime::input_latency::InputLatencyLog;
use appkit_shell::frame_pacer::WindowFramePacer;
use appkit_shell::window_input::ImeCursorAreaCache;
use winit::event_loop::EventLoopProxy;

use crate::app_event::AppEvent;
use crate::mouse::MouseState;

const STARTUP_LOG_ENV: &str = "TEXTORA_STARTUP_LOG";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RedrawState {
    Idle,
    Requested,
    DeferredWhileOccluded,
}

pub(crate) struct PreparedFontSystem {
    pub(crate) font_system: shaping::FontSystem,
    pub(crate) load_elapsed: Duration,
}

pub(crate) struct PlatformState {
    startup_file: Option<PathBuf>,
    startup_started_at: Instant,
    startup_log_path: Option<PathBuf>,
    font_preparation: Option<JoinHandle<PreparedFontSystem>>,
    event_loop_proxy: Option<EventLoopProxy<AppEvent>>,
    accessibility: Option<PlatformAccessibilityAdapter>,
    redraw_state: RedrawState,
    redraw_revision: u64,
    presenting: bool,
    last_cursor_visible: bool,
    mouse: MouseState,
    input_latency: Option<InputLatencyLog>,
    pub(crate) ime_cursor_area: ImeCursorAreaCache,
    pub(crate) frame_pacer: WindowFramePacer,
    #[cfg(target_os = "macos")]
    latency_critical_activity: Option<appkit_shell::macos::LatencyCriticalActivity>,
}

fn parse_startup_log_path(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let value = value?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

impl PlatformState {
    pub(crate) fn resolve_startup_log_path() -> Option<PathBuf> {
        parse_startup_log_path(std::env::var_os(STARTUP_LOG_ENV))
    }

    pub(crate) fn new(
        startup_file: Option<PathBuf>,
        startup_started_at: Instant,
        font_preparation: Option<JoinHandle<PreparedFontSystem>>,
        startup_log_path: Option<PathBuf>,
    ) -> Self {
        Self {
            startup_file,
            startup_started_at,
            startup_log_path,
            font_preparation,
            event_loop_proxy: None,
            accessibility: None,
            redraw_state: RedrawState::Requested,
            redraw_revision: 0,
            presenting: false,
            last_cursor_visible: true,
            mouse: MouseState::new(),
            input_latency: InputLatencyLog::open(),
            ime_cursor_area: ImeCursorAreaCache::default(),
            frame_pacer: WindowFramePacer::default(),
            #[cfg(target_os = "macos")]
            latency_critical_activity: None,
        }
    }

    fn write_startup_line(&self, line: &str) {
        eprintln!("{line}");
        let Some(path) = self.startup_log_path.as_ref() else {
            return;
        };
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{line}");
        }
    }

    pub(crate) fn log_startup_banner(&self, edition: crate::Edition) {
        let wall_ms =
            SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |duration| duration.as_millis());
        let line = format!(
            "[startup] === launch pid={} edition=\"{}\" wall_ms={} ===",
            std::process::id(),
            edition.name(),
            wall_ms
        );
        self.write_startup_line(&line);
    }

    pub(crate) fn log_startup_stage(
        &self,
        stage: &str,
        stage_elapsed: Duration,
        detail: std::fmt::Arguments<'_>,
    ) {
        let line = format!(
            "[startup] {stage} stage={:.2}ms total={:.2}ms{detail}",
            stage_elapsed.as_secs_f64() * 1_000.0,
            self.startup_elapsed().as_secs_f64() * 1_000.0
        );
        self.write_startup_line(&line);
    }

    pub(crate) fn log_startup_point(&self, label: &str, detail: std::fmt::Arguments<'_>) {
        let line = format!(
            "[startup] {label} total={:.2}ms{detail}",
            self.startup_elapsed().as_secs_f64() * 1_000.0
        );
        self.write_startup_line(&line);
    }

    pub(crate) fn startup_file(&self) -> Option<&Path> {
        self.startup_file.as_deref()
    }

    pub(crate) fn clear_startup_file(&mut self) {
        self.startup_file = None;
    }

    pub(crate) fn startup_elapsed(&self) -> Duration {
        self.startup_started_at.elapsed()
    }

    pub(crate) fn take_font_preparation(&mut self) -> Option<JoinHandle<PreparedFontSystem>> {
        self.font_preparation.take()
    }

    #[cfg(test)]
    pub(crate) fn has_font_preparation(&self) -> bool {
        self.font_preparation.is_some()
    }

    pub(crate) fn register_event_loop_proxy(&mut self, event_loop_proxy: EventLoopProxy<AppEvent>) {
        self.event_loop_proxy = Some(event_loop_proxy);
    }

    pub(crate) fn event_loop_proxy(&self) -> Option<&EventLoopProxy<AppEvent>> {
        self.event_loop_proxy.as_ref()
    }

    pub(crate) fn install_accessibility(&mut self, adapter: PlatformAccessibilityAdapter) {
        self.accessibility = Some(adapter);
    }

    pub(crate) fn accessibility(&self) -> Option<&PlatformAccessibilityAdapter> {
        self.accessibility.as_ref()
    }

    pub(crate) fn accessibility_mut(&mut self) -> Option<&mut PlatformAccessibilityAdapter> {
        self.accessibility.as_mut()
    }

    pub(crate) fn schedule_redraw(&mut self) {
        if self.redraw_state != RedrawState::DeferredWhileOccluded {
            self.redraw_state = RedrawState::Requested;
        }
        self.redraw_revision = self.redraw_revision.wrapping_add(1);
    }

    pub(crate) fn redraw_revision(&self) -> u64 {
        self.redraw_revision
    }

    pub(crate) fn begin_present(&mut self) -> bool {
        if self.presenting || self.redraw_state == RedrawState::DeferredWhileOccluded {
            return false;
        }
        self.presenting = true;
        self.clear_redraw();
        true
    }

    pub(crate) fn finish_present(&mut self) {
        self.presenting = false;
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn set_latency_critical(&mut self, focused: bool) {
        if focused {
            self.latency_critical_activity
                .get_or_insert_with(appkit_shell::macos::LatencyCriticalActivity::begin);
        } else {
            self.latency_critical_activity = None;
        }
    }

    pub(crate) fn clear_redraw(&mut self) {
        if self.redraw_state == RedrawState::Requested {
            self.redraw_state = RedrawState::Idle;
        }
    }

    pub(crate) fn redraw_pending(&self) -> bool {
        self.redraw_state == RedrawState::Requested
    }

    pub(crate) fn redraw_is_occluded(&self) -> bool {
        self.redraw_state == RedrawState::DeferredWhileOccluded
    }

    pub(crate) fn redraw_state(&self) -> RedrawState {
        self.redraw_state
    }

    pub(crate) fn defer_redraw_until_unoccluded(&mut self) {
        self.redraw_state = RedrawState::DeferredWhileOccluded;
        self.frame_pacer.stop();
    }

    pub(crate) fn handle_occlusion_changed(&mut self, occluded: bool, first_frame_pending: bool) {
        if occluded {
            self.defer_redraw_until_unoccluded();
            return;
        }
        if self.redraw_state == RedrawState::DeferredWhileOccluded || first_frame_pending {
            self.redraw_state = RedrawState::Requested;
        }
    }

    pub(crate) fn update_cursor_visibility(&mut self, visible: bool) {
        if visible != self.last_cursor_visible {
            self.record_presented_cursor_visibility(visible);
            self.schedule_redraw();
        }
    }

    pub(crate) fn record_presented_cursor_visibility(&mut self, visible: bool) {
        self.last_cursor_visible = visible;
    }

    pub(crate) fn mouse(&self) -> &MouseState {
        &self.mouse
    }

    pub(crate) fn mouse_mut(&mut self) -> &mut MouseState {
        &mut self.mouse
    }

    pub(crate) fn input_latency(&mut self) -> Option<&mut InputLatencyLog> {
        self.input_latency.as_mut()
    }

    pub(crate) fn input_latency_enabled(&self) -> bool {
        self.input_latency.is_some()
    }

    pub(crate) fn has_pending_input_sample(&self) -> bool {
        self.input_latency.as_ref().is_some_and(InputLatencyLog::has_pending_samples)
    }
}

#[cfg(test)]
mod tests {
    use super::{PlatformState, RedrawState, parse_startup_log_path};
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::Instant;

    fn test_platform_state() -> PlatformState {
        PlatformState::new(None, Instant::now(), None, None)
    }

    #[test]
    fn deferred_occluded_frame_does_not_spin_and_resumes_on_unoccluded() {
        let mut state = test_platform_state();
        state.clear_redraw();
        state.defer_redraw_until_unoccluded();

        assert_eq!(state.redraw_state(), RedrawState::DeferredWhileOccluded);
        assert!(!state.redraw_pending());
        state.schedule_redraw();
        assert!(!state.redraw_pending());

        state.handle_occlusion_changed(false, false);
        assert_eq!(state.redraw_state(), RedrawState::Requested);
        assert!(state.redraw_pending());
    }

    #[test]
    fn input_frame_that_restores_the_caret_does_not_request_an_extra_blink_frame() {
        let mut state = test_platform_state();
        state.update_cursor_visibility(false);
        state.clear_redraw();

        state.record_presented_cursor_visibility(true);
        state.update_cursor_visibility(true);

        assert!(!state.redraw_pending());
    }

    #[test]
    fn unoccluded_event_schedules_redraw_when_first_frame_is_pending() {
        let mut state = test_platform_state();
        state.clear_redraw();
        assert_eq!(state.redraw_state(), RedrawState::Idle);

        state.handle_occlusion_changed(false, true);
        assert_eq!(state.redraw_state(), RedrawState::Requested);
        assert!(state.redraw_pending());
    }

    #[test]
    fn occluded_event_defers_pending_redraw_without_losing_intent() {
        let mut state = test_platform_state();
        assert!(state.redraw_pending());

        state.handle_occlusion_changed(true, false);
        assert_eq!(state.redraw_state(), RedrawState::DeferredWhileOccluded);
        assert!(!state.redraw_pending());

        state.handle_occlusion_changed(false, false);
        assert_eq!(state.redraw_state(), RedrawState::Requested);
        assert!(state.redraw_pending());
    }

    #[test]
    fn startup_log_path_requires_explicit_non_empty_value() {
        assert_eq!(parse_startup_log_path(None), None);
        assert_eq!(parse_startup_log_path(Some(OsString::from(""))), None);
        assert_eq!(
            parse_startup_log_path(Some(OsString::from("/tmp/custom.log"))),
            Some(PathBuf::from("/tmp/custom.log"))
        );
    }
}
