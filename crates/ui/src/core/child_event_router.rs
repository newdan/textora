//! 复合控件的通用子控件事件路由状态机。
//!
//! 路由器只维护交互目标，不持有控件，也不负责坐标转换和 action 映射。容器负责命中测试，
//! 将得到的目标交给路由器，再按 [`ChildEventRoute`] 派发事件。

use crate::core::widget::{Event, EventCtx};

/// 焦点遍历方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDirection {
    Forward,
    Backward,
}

/// 根据当前目标和容器提供的候选顺序计算下一个焦点目标。
pub fn next_focus_target<T>(
    current_target: Option<T>,
    focusable_targets: &[T],
    direction: FocusDirection,
) -> Option<T>
where
    T: Copy + Eq,
{
    if focusable_targets.is_empty() {
        return None;
    }

    let current_index = current_target
        .and_then(|focused| focusable_targets.iter().position(|target| *target == focused));
    let next_index = match (current_index, direction) {
        (Some(0), FocusDirection::Backward) | (None, FocusDirection::Backward) => {
            focusable_targets.len() - 1
        }
        (Some(index), FocusDirection::Backward) => index - 1,
        (Some(index), FocusDirection::Forward) => (index + 1) % focusable_targets.len(),
        (None, FocusDirection::Forward) => 0,
    };
    Some(focusable_targets[next_index])
}

/// 单次事件的路由结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChildEventRoute<T> {
    /// 接收原始事件的唯一目标。
    pub primary_target: Option<T>,
    /// Hover 目标切换时，需要先接收 `PointerLeave` 的旧目标。
    pub pointer_leave_target: Option<T>,
    /// `InteractionCancel` 必须广播给容器内的全部子控件。
    pub broadcast: bool,
    /// 路由器内部的捕获或 Hover 状态是否发生变化。
    pub state_changed: bool,
}

/// 执行一次路由计划后的通用结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildEventDispatch<A> {
    pub action: Option<A>,
    pub broadcast: bool,
    pub state_changed: bool,
}

/// 执行 [`ChildEventRoute`] 描述的通用派发顺序。
///
/// 取消事件广播给 `broadcast_targets`；Hover 切换时先向旧目标发送 `PointerLeave`，并保留
/// 新目标设置的光标提示；原始事件只发送给唯一主目标。主目标 action 的优先级高于旧
/// Hover 目标产生的 action。
pub fn dispatch_child_event_route<T, A>(
    route: ChildEventRoute<T>,
    event: &Event,
    broadcast_targets: impl IntoIterator<Item = T>,
    ctx: &mut EventCtx,
    mut dispatch: impl FnMut(T, &Event, &mut EventCtx) -> Option<A>,
) -> ChildEventDispatch<A>
where
    T: Copy,
{
    if route.broadcast {
        let mut first_action = None;
        for target in broadcast_targets {
            if let Some(action) = dispatch(target, event, ctx)
                && first_action.is_none()
            {
                first_action = Some(action);
            }
        }
        return ChildEventDispatch {
            action: first_action,
            broadcast: true,
            state_changed: route.state_changed,
        };
    }

    let pointer_leave_action = route.pointer_leave_target.and_then(|target| {
        let saved_cursor_hint = ctx.cursor_hint;
        let action = dispatch(target, &Event::PointerLeave, ctx);
        ctx.cursor_hint = saved_cursor_hint;
        action
    });
    let primary_action = route.primary_target.and_then(|target| dispatch(target, event, ctx));
    ChildEventDispatch {
        action: primary_action.or(pointer_leave_action),
        broadcast: false,
        state_changed: route.state_changed,
    }
}

impl<T> ChildEventRoute<T> {
    const fn unchanged(primary_target: Option<T>) -> Self {
        Self { primary_target, pointer_leave_target: None, broadcast: false, state_changed: false }
    }
}

/// 容器级子控件事件路由器。
///
/// `T` 是容器内部稳定的目标标识，可以是 `WidgetId`、子控件枚举或索引。键盘/IME 始终
/// 路由到唯一焦点目标；指针事件路由到命中目标或捕获目标；Hover 与捕获互相独立。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildEventRouter<T> {
    focused_target: Option<T>,
    pointer_capture_target: Option<T>,
    hovered_target: Option<T>,
}

impl<T> Default for ChildEventRouter<T> {
    fn default() -> Self {
        Self { focused_target: None, pointer_capture_target: None, hovered_target: None }
    }
}

impl<T> ChildEventRouter<T>
where
    T: Copy + Eq,
{
    pub fn focused_target(&self) -> Option<T> {
        self.focused_target
    }

    pub fn pointer_capture_target(&self) -> Option<T> {
        self.pointer_capture_target
    }

    pub fn hovered_target(&self) -> Option<T> {
        self.hovered_target
    }

    pub fn set_focused_target(&mut self, focused_target: Option<T>) {
        self.focused_target = focused_target;
    }

    /// 同步由子控件自身建立的指针捕获，例如嵌套拖拽控件已经开始交互时。
    pub fn set_pointer_capture_target(&mut self, pointer_capture_target: Option<T>) {
        self.pointer_capture_target = pointer_capture_target;
    }

    /// 在容器提供的当前可见且可用目标之间循环焦点。
    pub fn cycle_focus(&mut self, focusable_targets: &[T], direction: FocusDirection) -> Option<T> {
        if focusable_targets.is_empty() {
            self.focused_target = None;
            return None;
        }

        let next_target = next_focus_target(self.focused_target, focusable_targets, direction)
            .expect("non-empty focus candidate list must produce a next target");
        self.focused_target = Some(next_target);
        Some(next_target)
    }

    /// 清空瞬时交互状态，保留键盘焦点。
    pub fn clear_interactions(&mut self) -> bool {
        let had_pointer_capture = self.pointer_capture_target.take().is_some();
        let had_hover = self.hovered_target.take().is_some();
        had_pointer_capture || had_hover
    }

    pub fn is_capturing(&self) -> bool {
        self.pointer_capture_target.is_some()
    }

    /// 根据事件类型和容器命中结果计算派发目标。
    ///
    /// 非指针事件的 `hit_target` 应传 `None`；路由器不会自行执行命中测试。
    pub fn route_event(&mut self, event: &Event, hit_target: Option<T>) -> ChildEventRoute<T> {
        match event {
            Event::KeyDown(..)
            | Event::ImePreedit { .. }
            | Event::ImeCommit(..)
            | Event::ImeEnable
            | Event::ImeDisable => ChildEventRoute::unchanged(self.focused_target),
            Event::MouseDown { .. } => self.route_mouse_down(hit_target),
            Event::MouseMove { .. } => self.route_mouse_move(hit_target),
            Event::MouseUp { .. } => {
                let primary_target = self.pointer_capture_target.take();
                ChildEventRoute {
                    primary_target,
                    pointer_leave_target: None,
                    broadcast: false,
                    state_changed: primary_target.is_some(),
                }
            }
            Event::Wheel { .. } => {
                ChildEventRoute::unchanged(self.pointer_capture_target.or(hit_target))
            }
            Event::PointerLeave => {
                let pointer_leave_target = self.hovered_target.take();
                ChildEventRoute {
                    primary_target: None,
                    pointer_leave_target,
                    broadcast: false,
                    state_changed: pointer_leave_target.is_some(),
                }
            }
            Event::InteractionCancel => {
                let state_changed = self.clear_interactions();
                ChildEventRoute {
                    primary_target: None,
                    pointer_leave_target: None,
                    broadcast: true,
                    state_changed,
                }
            }
        }
    }

    fn route_mouse_down(&mut self, hit_target: Option<T>) -> ChildEventRoute<T> {
        let previous_capture = self.pointer_capture_target;
        let previous_hover = self.hovered_target;
        self.pointer_capture_target = hit_target;
        self.hovered_target = hit_target;
        ChildEventRoute {
            primary_target: hit_target,
            pointer_leave_target: previous_hover.filter(|previous| Some(*previous) != hit_target),
            broadcast: false,
            state_changed: previous_capture != hit_target || previous_hover != hit_target,
        }
    }

    fn route_mouse_move(&mut self, hit_target: Option<T>) -> ChildEventRoute<T> {
        if let Some(captured_target) = self.pointer_capture_target {
            return ChildEventRoute::unchanged(Some(captured_target));
        }

        let previous_hover = self.hovered_target;
        self.hovered_target = hit_target;
        ChildEventRoute {
            primary_target: hit_target,
            pointer_leave_target: previous_hover.filter(|previous| Some(*previous) != hit_target),
            broadcast: false,
            state_changed: previous_hover != hit_target,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ChildEventRouter, FocusDirection, dispatch_child_event_route};
    use crate::core::widget::{Event, EventCtx, KeyCode, Modifiers, MouseButton};
    use winit::window::CursorIcon;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Target {
        First,
        Second,
        Disabled,
    }

    impl Target {
        const ALL: [Self; 3] = [Self::First, Self::Second, Self::Disabled];
    }

    #[test]
    fn keyboard_and_ime_route_only_to_the_focused_target() {
        let mut router = ChildEventRouter::default();
        router.set_focused_target(Some(Target::Second));

        let key_route = router
            .route_event(&Event::KeyDown(KeyCode::Char('x'), Modifiers::NONE), Some(Target::First));
        let ime_route =
            router.route_event(&Event::ImeCommit("输入".to_owned()), Some(Target::First));

        assert_eq!(key_route.primary_target, Some(Target::Second));
        assert_eq!(ime_route.primary_target, Some(Target::Second));
        assert_eq!(key_route.pointer_leave_target, None);
        assert!(!key_route.broadcast);
    }

    #[test]
    fn focus_cycle_uses_only_the_candidates_supplied_by_the_container() {
        let mut router = ChildEventRouter::default();
        router.set_focused_target(Some(Target::First));
        let candidates = [Target::First, Target::Second];

        assert_eq!(router.cycle_focus(&candidates, FocusDirection::Forward), Some(Target::Second));
        assert_eq!(router.cycle_focus(&candidates, FocusDirection::Forward), Some(Target::First));
        assert_eq!(router.cycle_focus(&candidates, FocusDirection::Backward), Some(Target::Second));
        assert_ne!(router.focused_target(), Some(Target::Disabled));
    }

    #[test]
    fn pointer_capture_owns_move_and_release_until_mouse_up() {
        let mut router = ChildEventRouter::default();
        let down = Event::MouseDown { px: 1.0, py: 1.0, button: MouseButton::Left };
        let movement = Event::MouseMove { px: 20.0, py: 20.0 };
        let up = Event::MouseUp { px: 20.0, py: 20.0, button: MouseButton::Left };

        assert_eq!(
            router.route_event(&down, Some(Target::First)).primary_target,
            Some(Target::First)
        );
        assert_eq!(router.pointer_capture_target(), Some(Target::First));
        assert_eq!(
            router.route_event(&movement, Some(Target::Second)).primary_target,
            Some(Target::First)
        );
        assert_eq!(
            router.route_event(&up, Some(Target::Second)).primary_target,
            Some(Target::First)
        );
        assert_eq!(router.pointer_capture_target(), None);
    }

    #[test]
    fn externally_established_capture_routes_wheel_and_release_without_mouse_down() {
        let mut router = ChildEventRouter::default();
        router.set_pointer_capture_target(Some(Target::First));
        let wheel = Event::Wheel { dx: 0.0, dy: 1.0, px: 20.0, py: 20.0 };
        let up = Event::MouseUp { px: 20.0, py: 20.0, button: MouseButton::Left };

        assert_eq!(
            router.route_event(&wheel, Some(Target::Second)).primary_target,
            Some(Target::First)
        );
        assert_eq!(
            router.route_event(&up, Some(Target::Second)).primary_target,
            Some(Target::First)
        );
        assert_eq!(router.pointer_capture_target(), None);
    }

    #[test]
    fn hover_transition_reports_the_previous_target_once() {
        let mut router = ChildEventRouter::default();
        let movement = Event::MouseMove { px: 1.0, py: 1.0 };

        let first = router.route_event(&movement, Some(Target::First));
        let second = router.route_event(&movement, Some(Target::Second));
        let unchanged = router.route_event(&movement, Some(Target::Second));

        assert_eq!(first.pointer_leave_target, None);
        assert_eq!(second.pointer_leave_target, Some(Target::First));
        assert_eq!(second.primary_target, Some(Target::Second));
        assert_eq!(unchanged.pointer_leave_target, None);
    }

    #[test]
    fn interaction_cancel_clears_pointer_state_and_requests_broadcast() {
        let mut router = ChildEventRouter::default();
        let down = Event::MouseDown { px: 1.0, py: 1.0, button: MouseButton::Left };
        let _ = router.route_event(&down, Some(Target::First));

        let cancel = router.route_event(&Event::InteractionCancel, None);

        assert!(cancel.broadcast);
        assert!(cancel.state_changed);
        assert_eq!(router.pointer_capture_target(), None);
        assert_eq!(router.hovered_target(), None);
    }

    #[test]
    fn generic_dispatch_sends_pointer_leave_before_primary_and_preserves_primary_cursor() {
        let mut router = ChildEventRouter::default();
        let movement = Event::MouseMove { px: 1.0, py: 1.0 };
        let _ = router.route_event(&movement, Some(Target::First));
        let route = router.route_event(&movement, Some(Target::Second));
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);
        let mut calls = Vec::new();

        let dispatch = dispatch_child_event_route(
            route,
            &movement,
            Target::ALL,
            &mut ctx,
            |target, event, ctx| {
                calls.push((target, matches!(event, Event::PointerLeave)));
                if matches!(event, Event::PointerLeave) {
                    ctx.cursor_hint = Some(CursorIcon::Crosshair);
                    return Some("leave");
                }
                ctx.cursor_hint = Some(CursorIcon::Pointer);
                Some("primary")
            },
        );

        assert_eq!(calls, vec![(Target::First, true), (Target::Second, false)]);
        assert_eq!(dispatch.action, Some("primary"));
        assert_eq!(ctx.cursor_hint, Some(CursorIcon::Pointer));
    }

    #[test]
    fn generic_dispatch_broadcasts_cancel_and_keeps_the_first_action() {
        let mut router = ChildEventRouter::default();
        let down = Event::MouseDown { px: 1.0, py: 1.0, button: MouseButton::Left };
        let _ = router.route_event(&down, Some(Target::First));
        let cancel = Event::InteractionCancel;
        let route = router.route_event(&cancel, None);
        let theme = crate::theme::test_theme();
        let mut ctx = EventCtx::new(&theme, 1.0);
        let mut calls = Vec::new();

        let dispatch = dispatch_child_event_route(
            route,
            &cancel,
            [Target::First, Target::Second],
            &mut ctx,
            |target, _, _| {
                calls.push(target);
                Some(target)
            },
        );

        assert_eq!(calls, vec![Target::First, Target::Second]);
        assert_eq!(dispatch.action, Some(Target::First));
        assert!(dispatch.broadcast);
        assert!(dispatch.state_changed);
    }
}
