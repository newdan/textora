use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum EventPumpState {
    #[default]
    Idle,
    Draining,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrainStart {
    Started,
    AlreadyDraining,
}

/// 保证 action 按 FIFO 顺序、且不经同步递归执行的事件队列。
#[derive(Debug)]
pub(crate) struct EventPump<Action> {
    pending_actions: VecDeque<Action>,
    state: EventPumpState,
}

impl<Action> EventPump<Action> {
    pub(crate) fn enqueue(&mut self, action: Action) {
        self.pending_actions.push_back(action);
    }

    pub(crate) fn start_draining(&mut self) -> DrainStart {
        if self.state == EventPumpState::Draining {
            return DrainStart::AlreadyDraining;
        }
        self.state = EventPumpState::Draining;
        DrainStart::Started
    }

    pub(crate) fn next_action(&mut self) -> Option<Action> {
        self.pending_actions.pop_front()
    }

    pub(crate) fn finish_draining(&mut self) {
        debug_assert!(self.pending_actions.is_empty(), "event pump must drain all queued actions");
        self.state = EventPumpState::Idle;
    }
}

impl<Action> Default for EventPump<Action> {
    fn default() -> Self {
        Self { pending_actions: VecDeque::new(), state: EventPumpState::Idle }
    }
}

#[cfg(test)]
mod tests {
    use super::{DrainStart, EventPump};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum TestAction {
        Outer,
        Inner,
    }

    #[test]
    fn action_enqueued_by_an_effect_waits_until_the_current_action_finishes() {
        let mut pump = EventPump::default();
        let mut execution_order = Vec::new();
        pump.enqueue(TestAction::Outer);

        assert_eq!(pump.start_draining(), DrainStart::Started);
        assert_eq!(pump.next_action(), Some(TestAction::Outer));
        execution_order.push("outer-effect-one");
        pump.enqueue(TestAction::Inner);
        execution_order.push("outer-effect-two");
        assert_eq!(pump.start_draining(), DrainStart::AlreadyDraining);
        assert_eq!(pump.next_action(), Some(TestAction::Inner));
        execution_order.push("inner-action");
        assert_eq!(pump.next_action(), None);
        pump.finish_draining();

        assert_eq!(execution_order, vec!["outer-effect-one", "outer-effect-two", "inner-action"]);
    }

    #[test]
    fn actions_keep_fifo_order_across_one_drain_cycle() {
        let mut pump = EventPump::default();
        pump.enqueue(TestAction::Outer);
        pump.enqueue(TestAction::Inner);

        assert_eq!(pump.start_draining(), DrainStart::Started);
        assert_eq!(pump.next_action(), Some(TestAction::Outer));
        assert_eq!(pump.next_action(), Some(TestAction::Inner));
        assert_eq!(pump.next_action(), None);
        pump.finish_draining();
        assert_eq!(pump.start_draining(), DrainStart::Started);
    }
}
