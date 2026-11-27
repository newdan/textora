//! Display-paced rendering shared by desktop products.

use std::time::{Duration, Instant};

use winit::event::{Ime, WindowEvent};
use winit::event_loop::EventLoopProxy;
use winit::window::Window;

use crate::ShellEvent;

const DEFAULT_REFRESH_RATE_HZ: f64 = 60.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PacerState {
    #[default]
    Idle,
    WaitingForTick,
    SubmittedThisCycle {
        pending_followup: bool,
    },
    Continuous {
        dirty: bool,
    },
}

/// The request describes when a visible change is eligible to be painted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FrameRequest {
    Immediate,
    Deferred,
    Continuous,
}

pub fn input_frame_request(event: &WindowEvent, pointer_captured: bool) -> Option<FrameRequest> {
    match event {
        WindowEvent::Ime(Ime::Preedit(text, _) | Ime::Commit(text)) if !text.is_empty() => {
            Some(FrameRequest::Immediate)
        }
        WindowEvent::Ime(Ime::Preedit(..)) => Some(FrameRequest::Deferred),
        WindowEvent::KeyboardInput { event, .. } if event.state.is_pressed() => {
            Some(FrameRequest::Immediate)
        }
        WindowEvent::MouseWheel { .. } | WindowEvent::PinchGesture { .. } => {
            Some(FrameRequest::Continuous)
        }
        WindowEvent::CursorMoved { .. } if pointer_captured => Some(FrameRequest::Continuous),
        _ => None,
    }
}

#[derive(Default)]
pub struct FramePacer {
    state: PacerState,
    // Leave the clock running for one final quiet tick while allowing input
    // arriving in the new refresh cycle to present immediately.
    cooling_down: bool,
}

impl FramePacer {
    pub fn request(&mut self, request: FrameRequest) -> bool {
        self.cooling_down = false;
        match (&mut self.state, request) {
            (_, FrameRequest::Continuous) => self.state = PacerState::Continuous { dirty: true },
            (PacerState::Idle | PacerState::WaitingForTick, FrameRequest::Immediate) => {
                self.state = PacerState::SubmittedThisCycle { pending_followup: false };
                return true;
            }
            (PacerState::Idle, FrameRequest::Deferred) => self.state = PacerState::WaitingForTick,
            (PacerState::SubmittedThisCycle { pending_followup }, _) => *pending_followup = true,
            (PacerState::Continuous { dirty }, _) => *dirty = true,
            (PacerState::WaitingForTick, FrameRequest::Deferred) => {}
        }
        false
    }

    pub fn tick(&mut self, animation_active: bool) -> bool {
        let dirty = match self.state {
            PacerState::Idle => false,
            PacerState::WaitingForTick => true,
            PacerState::SubmittedThisCycle { pending_followup } => pending_followup,
            PacerState::Continuous { dirty } => dirty,
        };
        if animation_active {
            self.state = PacerState::Continuous { dirty: false };
            self.cooling_down = false;
            return true;
        }
        if dirty {
            self.state = PacerState::SubmittedThisCycle { pending_followup: false };
            self.cooling_down = false;
            return true;
        }
        self.cooling_down = self.state != PacerState::Idle;
        self.state = PacerState::Idle;
        false
    }

    pub fn is_active(&self) -> bool {
        self.state != PacerState::Idle || self.cooling_down
    }
}

/// Supplies either a native display tick or a deadline for the event loop.
pub trait DisplayPacingBackend {
    fn set_active(&mut self, active: bool, now: Instant);
    fn take_tick(&mut self, now: Instant) -> Option<Instant>;
    fn deadline(&self) -> Option<Instant>;
}

struct FixedInterval {
    interval: Duration,
    next_tick: Option<Instant>,
}

impl FixedInterval {
    fn new(interval: Duration) -> Self {
        Self { interval, next_tick: None }
    }
}

impl DisplayPacingBackend for FixedInterval {
    fn set_active(&mut self, active: bool, now: Instant) {
        if active {
            self.next_tick.get_or_insert(now + self.interval);
        } else {
            self.next_tick = None;
        }
    }

    fn take_tick(&mut self, now: Instant) -> Option<Instant> {
        let deadline = self.next_tick.filter(|deadline| now >= *deadline)?;
        let offset = (now - deadline).as_nanos() % self.interval.as_nanos();
        let tick = now - Duration::from_nanos(offset as u64);
        self.next_tick = Some(tick + self.interval);
        Some(tick)
    }

    fn deadline(&self) -> Option<Instant> {
        self.next_tick
    }
}

pub struct WindowFramePacer {
    pacer: FramePacer,
    clock: Box<dyn DisplayPacingBackend>,
    last_frame_started_at: Option<Instant>,
}

impl Default for WindowFramePacer {
    fn default() -> Self {
        Self::with_clock(FixedInterval::new(Duration::from_secs_f64(1.0 / DEFAULT_REFRESH_RATE_HZ)))
    }
}

impl WindowFramePacer {
    fn with_clock(clock: impl DisplayPacingBackend + 'static) -> Self {
        Self { pacer: FramePacer::default(), clock: Box::new(clock), last_frame_started_at: None }
    }

    pub fn for_window(window: &Window, proxy: Option<EventLoopProxy<ShellEvent>>) -> Self {
        #[cfg(target_os = "macos")]
        if let Some(proxy) = proxy
            && let Some(link) = crate::macos::WindowDisplayLink::new(window, proxy)
        {
            return Self::with_clock(link);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = proxy;
        let refresh_hz = window
            .current_monitor()
            .and_then(|monitor| monitor.refresh_rate_millihertz())
            .filter(|rate| *rate > 0)
            .map_or(DEFAULT_REFRESH_RATE_HZ, |rate| f64::from(rate) / 1000.0);
        Self::with_clock(FixedInterval::new(Duration::from_secs_f64(1.0 / refresh_hz)))
    }

    pub fn request(&mut self, request: FrameRequest, now: Instant) -> bool {
        let render = self.pacer.request(request);
        self.clock.set_active(true, now);
        if render {
            self.last_frame_started_at = Some(now);
        }
        render
    }

    pub fn take_tick(&mut self, now: Instant, animation_active: bool) -> bool {
        let Some(tick) = self.clock.take_tick(now) else {
            return false;
        };
        // winit can deliver an input before a display tick that AppKit already queued.
        if !self.pacer.is_active() || self.last_frame_started_at.is_some_and(|start| tick <= start)
        {
            return false;
        }
        let render = self.pacer.tick(animation_active);
        self.clock.set_active(self.pacer.is_active(), now);
        if render {
            self.last_frame_started_at = Some(now);
        }
        render
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.clock.deadline()
    }

    pub fn stop(&mut self) {
        self.pacer = FramePacer::default();
        self.last_frame_started_at = None;
        self.clock.set_active(false, Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_input_is_immediate_and_a_burst_coalesces_until_the_tick() {
        let mut pacer = FramePacer::default();
        assert!(pacer.request(FrameRequest::Immediate));
        assert!(!pacer.request(FrameRequest::Immediate));
        assert!(!pacer.request(FrameRequest::Immediate));
        assert!(pacer.tick(false));
        assert!(!pacer.tick(false));
        assert!(pacer.is_active());
        assert!(!pacer.tick(false));
        assert!(!pacer.is_active());
        assert!(pacer.request(FrameRequest::Immediate));
    }

    #[test]
    fn deferred_preedit_clear_renders_on_a_tick_and_continuous_animation_keeps_pacing() {
        let mut pacer = FramePacer::default();
        assert!(!pacer.request(FrameRequest::Deferred));
        assert!(pacer.tick(false));
        assert!(!pacer.request(FrameRequest::Continuous));
        assert!(pacer.tick(true));
        assert!(pacer.tick(true));
        assert!(!pacer.tick(false));
        assert!(!pacer.tick(false));
        assert!(!pacer.is_active());
    }

    #[test]
    fn fixed_timer_skips_missed_cycles_without_catchup_frames() {
        let start = std::time::Instant::now();
        let period = std::time::Duration::from_millis(10);
        let mut backend = FixedInterval::new(period);
        backend.set_active(true, start);
        assert_eq!(backend.take_tick(start + period / 2), None);
        assert_eq!(backend.take_tick(start + period * 5), Some(start + period * 5));
        assert_eq!(backend.take_tick(start + period * 5), None);
        assert_eq!(backend.deadline(), Some(start + period * 6));
        backend.set_active(false, start + period * 5);
        assert_eq!(backend.deadline(), None);
    }

    #[test]
    fn an_undelivered_tick_before_immediate_input_cannot_submit_a_second_frame() {
        let start = Instant::now();
        let interval = Duration::from_millis(10);
        let mut window = WindowFramePacer::with_clock(FixedInterval::new(interval));
        assert!(window.request(FrameRequest::Immediate, start));
        assert!(!window.take_tick(start + interval, false));

        let after_queued_tick = start + interval * 2 + Duration::from_millis(1);
        assert!(window.request(FrameRequest::Immediate, after_queued_tick));
        assert!(!window.request(FrameRequest::Immediate, after_queued_tick));
        assert!(!window.take_tick(after_queued_tick, false));
        assert!(window.take_tick(start + interval * 3, false));
    }
}
