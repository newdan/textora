use std::sync::Arc;

use crate::{ShellEffect, ShellEvent};

#[derive(Clone)]
pub struct ProductWakeHandle {
    wake: Arc<dyn Fn() -> Result<(), WakeError> + Send + Sync>,
}

impl ProductWakeHandle {
    pub fn new(event_loop_proxy: winit::event_loop::EventLoopProxy<ShellEvent>) -> Self {
        Self {
            wake: Arc::new(move || {
                event_loop_proxy.send_event(ShellEvent::ProductWake).map_err(|_| WakeError)
            }),
        }
    }

    pub fn wake(&self) -> Result<(), WakeError> {
        (self.wake)()
    }

    #[cfg(test)]
    pub(crate) fn from_callback(
        wake: impl Fn() -> Result<(), WakeError> + Send + Sync + 'static,
    ) -> Self {
        Self { wake: Arc::new(wake) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WakeError;

impl std::fmt::Display for WakeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("event loop is unavailable")
    }
}

impl std::error::Error for WakeError {}

pub trait ProductHost {
    fn start_background_services(&mut self, wake: ProductWakeHandle);
    fn drain_product_events(&mut self) -> ShellEffect;
    fn shutdown(&mut self);
}

#[cfg(test)]
mod tests {
    use super::{ProductHost, ProductWakeHandle, WakeError};
    use crate::ShellEffect;

    #[test]
    fn fake_host_exposes_only_shell_effects() {
        struct FakeHost {
            drained: bool,
            stopped: bool,
        }

        impl ProductHost for FakeHost {
            fn start_background_services(&mut self, _wake: ProductWakeHandle) {
                unreachable!("wake construction is covered separately");
            }

            fn drain_product_events(&mut self) -> ShellEffect {
                self.drained = true;
                ShellEffect::REDRAW
            }

            fn shutdown(&mut self) {
                self.stopped = true;
            }
        }

        let mut host = FakeHost { drained: false, stopped: false };
        assert_eq!(host.drain_product_events(), ShellEffect::REDRAW);
        host.shutdown();
        assert!(host.drained);
        assert!(host.stopped);
    }

    #[test]
    fn wake_error_is_stable_and_payload_free() {
        assert_eq!(WakeError.to_string(), "event loop is unavailable");
    }
}
