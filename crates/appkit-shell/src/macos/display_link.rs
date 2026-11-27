use std::cell::{Cell, OnceCell};
use std::time::{Duration, Instant};

use objc2::rc::{Retained, Weak};
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSWindow, NSWindowDidChangeScreenNotification};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSNotificationCenter, NSRunLoop, NSRunLoopCommonModes,
};
use objc2_quartz_core::{CACurrentMediaTime, CADisplayLink, CAFrameRateRange};
use winit::event_loop::EventLoopProxy;
use winit::window::Window;

use crate::ShellEvent;
use crate::frame_pacer::DisplayPacingBackend;

const MINIMUM_PREFERRED_REFRESH_HZ: f32 = 60.0;

struct DisplayLinkTargetState {
    proxy: EventLoopProxy<ShellEvent>,
    pending_tick: Cell<Option<Instant>>,
    link: OnceCell<Weak<CADisplayLink>>,
}

define_class!(
    // SAFETY: The target is installed on the main run loop and observes only its NSWindow.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DisplayLinkTargetState]
    struct DisplayLinkTarget;

    impl DisplayLinkTarget {
        #[unsafe(method(textoraDisplayLinkTick:))]
        fn on_tick(&self, link: &CADisplayLink) {
            // Use the display boundary even when AppKit delivers this callback late.
            let elapsed = Duration::try_from_secs_f64((CACurrentMediaTime() - link.timestamp()).max(0.0))
                .unwrap_or_default();
            let now = Instant::now();
            let tick = now.checked_sub(elapsed).unwrap_or(now);
            if self.ivars().pending_tick.replace(Some(tick)).is_none()
                && self.ivars().proxy.send_event(ShellEvent::DisplayTick).is_err()
            {
                self.ivars().pending_tick.set(None);
            }
        }

        #[unsafe(method(textoraDisplayLinkScreenChanged:))]
        fn on_screen_change(&self, notification: &NSNotification) {
            let Some(window) = notification.object().and_then(|object| object.downcast::<NSWindow>().ok())
            else {
                return;
            };
            if let Some(link) = self.ivars().link.get().and_then(Weak::load) {
                // A link created by NSWindow already follows the new screen.
                update_refresh_range(&link, &window);
            }
        }
    }
);

pub(crate) struct WindowDisplayLink {
    link: Retained<CADisplayLink>,
    target: Retained<DisplayLinkTarget>,
}

impl WindowDisplayLink {
    pub(crate) fn new(window: &Window, proxy: EventLoopProxy<ShellEvent>) -> Option<Self> {
        let main_thread = MainThreadMarker::new()?;
        let window = super::native_window(window)?;
        if !window.respondsToSelector(sel!(displayLinkWithTarget:selector:)) {
            return None;
        }
        let target = DisplayLinkTarget::alloc(main_thread).set_ivars(DisplayLinkTargetState {
            proxy,
            pending_tick: Cell::new(None),
            link: OnceCell::new(),
        });
        // SAFETY: NSObject's initializer is called once on this allocated target.
        let target: Retained<DisplayLinkTarget> = unsafe { msg_send![super(target), init] };
        // SAFETY: The selector belongs to target and accepts the display link supplied by AppKit.
        let link = unsafe {
            window.displayLinkWithTarget_selector(&target, sel!(textoraDisplayLinkTick:))
        };
        target.ivars().link.get_or_init(|| Weak::from_retained(&link));
        link.setPaused(true);
        update_refresh_range(&link, &window);
        // SAFETY: Both selectors are invoked on the main thread for this window's callbacks.
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &target,
                sel!(textoraDisplayLinkScreenChanged:),
                Some(NSWindowDidChangeScreenNotification),
                Some(&window),
            );
            link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes);
        }
        Some(Self { link, target })
    }
}

fn update_refresh_range(link: &CADisplayLink, window: &NSWindow) {
    let Some(refresh_hz) = window
        .screen()
        .map(|screen| screen.maximumFramesPerSecond() as f32)
        .filter(|rate| *rate > 0.0)
    else {
        return;
    };
    link.setPreferredFrameRateRange(CAFrameRateRange::new(
        refresh_hz.min(MINIMUM_PREFERRED_REFRESH_HZ),
        refresh_hz,
        refresh_hz,
    ));
}

impl DisplayPacingBackend for WindowDisplayLink {
    fn set_active(&mut self, active: bool, _now: Instant) {
        if self.link.isPaused() == active {
            self.link.setPaused(!active);
        }
        if !active {
            self.target.ivars().pending_tick.set(None);
        }
    }

    fn take_tick(&mut self, _now: Instant) -> Option<Instant> {
        self.target.ivars().pending_tick.take()
    }

    fn deadline(&self) -> Option<Instant> {
        None
    }
}

impl Drop for WindowDisplayLink {
    fn drop(&mut self) {
        self.link.invalidate();
        // SAFETY: target is still alive and was registered with this center.
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(&self.target) };
    }
}
