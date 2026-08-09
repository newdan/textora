//! 产品无关的 UI Action 队列与后台完成事件收件箱。

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, mpsc};

use crate::{ProductWakeHandle, WakeError};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum EventPumpState {
    #[default]
    Idle,
    Draining,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrainStart {
    Started,
    AlreadyDraining,
}

/// 保证 Action 按 FIFO 顺序、且不经同步递归执行的 UI 线程队列。
pub struct EventPump<Action> {
    pending_actions: VecDeque<Action>,
    state: EventPumpState,
}

impl<Action> EventPump<Action> {
    pub fn enqueue(&mut self, action: Action) {
        self.pending_actions.push_back(action);
    }

    pub fn start_draining(&mut self) -> DrainStart {
        if self.state == EventPumpState::Draining {
            return DrainStart::AlreadyDraining;
        }
        self.state = EventPumpState::Draining;
        DrainStart::Started
    }

    pub fn next_action(&mut self) -> Option<Action> {
        self.pending_actions.pop_front()
    }

    pub fn finish_draining(&mut self) {
        debug_assert!(self.pending_actions.is_empty(), "event pump must drain all queued actions");
        self.state = EventPumpState::Idle;
    }
}

impl<Action> Default for EventPump<Action> {
    fn default() -> Self {
        Self { pending_actions: VecDeque::new(), state: EventPumpState::Idle }
    }
}

struct ProductEventChannelState {
    wake_handle: OnceLock<ProductWakeHandle>,
    pending_event_count: AtomicUsize,
}

impl ProductEventChannelState {
    fn new() -> Self {
        Self { wake_handle: OnceLock::new(), pending_event_count: AtomicUsize::new(0) }
    }
}

/// 可跨线程克隆的产品完成事件发送端。
///
/// `send` 在 payload 成功入队后才发送无 payload 的 product wake。若 wake 失败，payload
/// 仍保留在 inbox 中；调用方不应重试同一事件。
pub struct ProductEventSender<Event> {
    sender: mpsc::Sender<Event>,
    state: Arc<ProductEventChannelState>,
}

impl<Event> Clone for ProductEventSender<Event> {
    fn clone(&self) -> Self {
        Self { sender: self.sender.clone(), state: Arc::clone(&self.state) }
    }
}

impl<Event> ProductEventSender<Event> {
    pub fn send(&self, event: Event) -> Result<(), ProductEventSendError> {
        self.state.pending_event_count.fetch_add(1, Ordering::Release);
        if self.sender.send(event).is_err() {
            self.state.pending_event_count.fetch_sub(1, Ordering::AcqRel);
            return Err(ProductEventSendError::ReceiverUnavailable);
        }

        let Some(wake_handle) = self.state.wake_handle.get() else {
            return Ok(());
        };
        wake_handle.wake().map_err(ProductEventSendError::Wake)
    }
}

/// 产品完成事件的 UI 线程接收端。
pub struct ProductEventInbox<Event> {
    receiver: mpsc::Receiver<Event>,
    state: Arc<ProductEventChannelState>,
}

impl<Event> ProductEventInbox<Event> {
    /// 注册唯一的产品唤醒句柄。注册前已入队的事件会立即触发一次 wake。
    pub fn register_wake_handle(
        &self,
        wake_handle: ProductWakeHandle,
    ) -> Result<(), ProductWakeRegistrationError> {
        self.state
            .wake_handle
            .set(wake_handle)
            .map_err(|_| ProductWakeRegistrationError::AlreadyRegistered)?;
        if self.state.pending_event_count.load(Ordering::Acquire) == 0 {
            return Ok(());
        }
        self.state
            .wake_handle
            .get()
            .expect("wake handle was registered immediately before use")
            .wake()
            .map_err(ProductWakeRegistrationError::Wake)
    }

    /// 排空调用时已经到达的事件，并保持 channel FIFO 顺序。
    pub fn drain(&self) -> Vec<Event> {
        let events: Vec<_> = self.receiver.try_iter().collect();
        self.state.pending_event_count.fetch_sub(events.len(), Ordering::AcqRel);
        events
    }
}

pub fn product_event_channel<Event>() -> (ProductEventSender<Event>, ProductEventInbox<Event>) {
    let (sender, receiver) = mpsc::channel();
    let state = Arc::new(ProductEventChannelState::new());
    (
        ProductEventSender { sender, state: Arc::clone(&state) },
        ProductEventInbox { receiver, state },
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductEventSendError {
    ReceiverUnavailable,
    Wake(WakeError),
}

impl fmt::Display for ProductEventSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReceiverUnavailable => {
                formatter.write_str("product event receiver is unavailable")
            }
            Self::Wake(error) => {
                write!(formatter, "product event was queued but wake failed: {error}")
            }
        }
    }
}

impl std::error::Error for ProductEventSendError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductWakeRegistrationError {
    AlreadyRegistered,
    Wake(WakeError),
}

impl fmt::Display for ProductWakeRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRegistered => {
                formatter.write_str("product wake handle is already registered")
            }
            Self::Wake(error) => {
                write!(formatter, "queued product events could not wake the event loop: {error}")
            }
        }
    }
}

impl std::error::Error for ProductWakeRegistrationError {}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::{DrainStart, EventPump, product_event_channel};
    use crate::ProductWakeHandle;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum TestAction {
        Outer,
        Inner,
    }

    #[test]
    fn action_pump_preserves_fifo_and_rejects_synchronous_reentry() {
        let mut pump = EventPump::default();
        pump.enqueue(TestAction::Outer);

        assert_eq!(pump.start_draining(), DrainStart::Started);
        assert_eq!(pump.next_action(), Some(TestAction::Outer));
        pump.enqueue(TestAction::Inner);
        assert_eq!(pump.start_draining(), DrainStart::AlreadyDraining);
        assert_eq!(pump.next_action(), Some(TestAction::Inner));
        assert_eq!(pump.next_action(), None);
        pump.finish_draining();
    }

    #[test]
    fn product_sender_enqueues_before_waking_and_inbox_preserves_fifo() {
        let (sender, inbox) = product_event_channel();
        let inbox = Arc::new(Mutex::new(inbox));
        let observed_events = Arc::new(Mutex::new(Vec::new()));
        let wake_count = Arc::new(AtomicUsize::new(0));
        let wake_inbox = Arc::clone(&inbox);
        let wake_observed_events = Arc::clone(&observed_events);
        let wake_counter = Arc::clone(&wake_count);

        inbox
            .lock()
            .expect("test inbox mutex should remain available")
            .register_wake_handle(ProductWakeHandle::from_callback(move || {
                wake_counter.fetch_add(1, Ordering::Relaxed);
                wake_observed_events
                    .lock()
                    .expect("test observation mutex should remain available")
                    .extend(
                        wake_inbox
                            .lock()
                            .expect("test inbox mutex should remain available")
                            .drain(),
                    );
                Ok(())
            }))
            .expect("test wake handle should register");

        sender.send("first").expect("test inbox should receive first event");
        sender.send("second").expect("test inbox should receive second event");

        assert_eq!(wake_count.load(Ordering::Relaxed), 2);
        assert_eq!(
            observed_events
                .lock()
                .expect("test observation mutex should remain available")
                .as_slice(),
            ["first", "second"]
        );
    }

    #[test]
    fn registering_wake_after_send_wakes_for_queued_events() {
        let (sender, inbox) = product_event_channel();
        sender.send(7).expect("event should queue before wake registration");
        let wake_count = Arc::new(AtomicUsize::new(0));
        let registered_wake_count = Arc::clone(&wake_count);

        inbox
            .register_wake_handle(ProductWakeHandle::from_callback(move || {
                registered_wake_count.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }))
            .expect("wake handle should register");

        assert_eq!(wake_count.load(Ordering::Relaxed), 1);
        assert_eq!(inbox.drain(), vec![7]);
    }
}
