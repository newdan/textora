//! notora 产品事件与 editor runtime 输入门控。

use appkit_shell::ShellEvent;
use appkit_shell::canvas_viewport::{
    CanvasWheelDelta, CanvasWheelMode, canvas_pinch_viewport_action, canvas_wheel_viewport_action,
};
use appkit_shell::editor_runtime::{EditorFocus, EditorInputContext};
use appkit_shell::window_input::{scroll_delta_pixels, ui_modifiers, winit_key_to_keycode};
use winit::application::ApplicationHandler;
use winit::event::{
    ElementState, Ime, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::ModifiersState;
use winit::window::WindowId;

use crate::NotoraApp;
use crate::action::NotoraAction;
use crate::runtime::NotoraRuntime;
use crate::{FocusTarget, NotoraState, OverlayState};
use ui::core::Modifiers;

fn canvas_wheel_action(
    delta: MouseScrollDelta,
    modifiers: ModifiersState,
    pointer_position: (f32, f32),
) -> Option<appkit_shell::canvas_viewport::CanvasViewportAction> {
    use ui::canvas::CanvasPoint;

    let delta = match delta {
        MouseScrollDelta::LineDelta(horizontal_delta, vertical_delta) => {
            CanvasWheelDelta::Lines(CanvasPoint::new(horizontal_delta, vertical_delta))
        }
        MouseScrollDelta::PixelDelta(position) => {
            CanvasWheelDelta::Pixels(CanvasPoint::new(position.x as f32, position.y as f32))
        }
    };
    let mode = if modifiers.super_key() || modifiers.control_key() {
        CanvasWheelMode::Zoom
    } else if modifiers.shift_key() {
        CanvasWheelMode::PanHorizontally
    } else {
        CanvasWheelMode::Pan
    };
    canvas_wheel_viewport_action(
        delta,
        mode,
        CanvasPoint::new(pointer_position.0, pointer_position.1),
    )
}

fn canvas_pinch_action(
    delta: f64,
    pointer_position: (f32, f32),
) -> Option<appkit_shell::canvas_viewport::CanvasViewportAction> {
    canvas_pinch_viewport_action(
        delta,
        ui::canvas::CanvasPoint::new(pointer_position.0, pointer_position.1),
    )
}

/// 根据产品焦点和 overlay 状态构造 runtime 输入上下文。
pub fn editor_input_context(state: &NotoraState, window_focused: bool) -> EditorInputContext {
    let editor_is_focused = state.layout.focus_target == FocusTarget::Editor && window_focused;
    EditorInputContext {
        focus: if editor_is_focused { EditorFocus::Active } else { EditorFocus::Inactive },
        modal_blocked: state.layout.overlay != OverlayState::None,
    }
}

impl ApplicationHandler<ShellEvent> for NotoraApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.runtime_mut().handle_resumed(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ShellEvent) {
        self.runtime_mut().handle_user_event(event_loop, event);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        self.runtime_mut().handle_window_event(event_loop, window_id, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.runtime_mut().handle_about_to_wait(event_loop);
    }
}

impl NotoraRuntime {
    fn handle_resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.resume(event_loop) {
            eprintln!("notora window initialization failed: {error}");
            event_loop.exit();
        }
    }

    fn handle_user_event(&mut self, _event_loop: &ActiveEventLoop, event: ShellEvent) {
        match event {
            ShellEvent::ProductWake => self.drain_product_events(),
            ShellEvent::SaveResultsReady => {
                self.drain_runtime_save_completions();
                self.request_window_redraw();
            }
            ShellEvent::StartBackgroundServices
            | ShellEvent::ReshapeResultsReady
            | ShellEvent::FileSafetyResultsReady => self.request_window_redraw(),
            ShellEvent::Accessibility(_) => {}
        }
    }

    fn handle_window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.shutdown();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => self.resize_window(size.width, size.height),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.set_scale_factor(scale_factor)
            }
            WindowEvent::Focused(focused) => self.set_window_focused(focused),
            WindowEvent::ThemeChanged(system_appearance) if self.follows_system_theme() => {
                self.rebuild_theme_for_system_appearance(system_appearance);
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.editor_runtime_mut().set_input_modifiers(modifiers.state());
            }
            WindowEvent::CursorMoved { position, .. } => {
                let (px, py) = (position.x as f32, position.y as f32);
                self.set_pointer_position(px, py);
                let pointer_event = ui::Event::MouseMove { px, py };
                self.route_pointer_event(&pointer_event);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(button) = map_mouse_button(button) else {
                    return;
                };
                let (px, py) = self.pointer_position();
                let product_event = match state {
                    ElementState::Pressed => ui::Event::MouseDown { px, py, button },
                    ElementState::Released => ui::Event::MouseUp { px, py, button },
                };
                self.route_pointer_event(&product_event);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (px, py) = self.pointer_position();
                let (dx, dy) = scroll_delta_pixels(&delta, self.shell_layout().dpi * 16.0);
                let product_consumed =
                    self.route_product_event(&ui::Event::Wheel { dx, dy, px, py });
                if product_consumed {
                    return;
                }
                let modifiers = self.editor_runtime_mut().input_modifiers();
                if let Some(action) = canvas_wheel_action(delta, modifiers, (px, py))
                    && self.apply_canvas_viewport_action_at(px, py, action)
                {
                    return;
                }
                self.scroll_editor(px, py, -dy);
            }
            WindowEvent::PinchGesture { delta, .. } => {
                let (px, py) = self.pointer_position();
                if let Some(action) = canvas_pinch_action(delta, (px, py)) {
                    let _ = self.apply_canvas_viewport_action_at(px, py, action);
                }
            }
            WindowEvent::DroppedFile(path) => {
                self.receive_system_open_paths(vec![path]);
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                let product_consumed =
                    self.route_product_event(&ui::Event::ImePreedit { text: text.clone(), cursor });
                if should_forward_input_to_editor(
                    self.state().layout.focus_target,
                    product_consumed,
                ) {
                    let _ = self.update_editor_preedit(text, cursor);
                }
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                let product_consumed =
                    self.route_product_event(&ui::Event::ImeCommit(text.clone()));
                if should_forward_input_to_editor(
                    self.state().layout.focus_target,
                    product_consumed,
                ) {
                    self.commit_editor_text(text);
                }
            }
            WindowEvent::Ime(Ime::Enabled) => {
                let _ = self.route_product_event(&ui::Event::ImeEnable);
            }
            WindowEvent::Ime(Ime::Disabled) => {
                let _ = self.route_product_event(&ui::Event::ImeDisable);
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let Some(key_code) =
                    winit_key_to_keycode(&event.logical_key, event.text.as_deref())
                else {
                    return;
                };
                let modifiers = ui_modifiers(self.editor_runtime_mut().input_modifiers());
                self.handle_key_input(key_code, modifiers);
            }
            WindowEvent::RedrawRequested => match self.render() {
                Ok(()) => {
                    self.record_first_frame_visible();
                    self.restore_session_after_first_frame();
                }
                Err(error) => eprintln!("notora frame rendering failed: {error}"),
            },
            _ => {}
        }
    }

    fn handle_about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.process_due_scheduled_work();
        if self.take_redraw_request() {
            self.request_window_redraw();
        }
        event_loop.set_control_flow(
            self.next_deadline().map(ControlFlow::WaitUntil).unwrap_or(ControlFlow::Wait),
        );
    }
}

impl NotoraRuntime {
    pub(crate) fn handle_key_input(&mut self, key_code: ui::KeyCode, modifiers: Modifiers) {
        let key_event = ui::Event::KeyDown(key_code, modifiers);
        if key_code == ui::KeyCode::Escape {
            if !self.route_product_event(&key_event) {
                self.dispatch_action(NotoraAction::EscapePressed);
            }
            return;
        }
        if self.state().layout.overlay != OverlayState::None {
            let _ = self.route_product_event(&key_event);
            return;
        }
        if is_open_external_shortcut(key_code, modifiers) {
            self.request_external_file_dialog();
            return;
        }
        if is_open_settings_shortcut(key_code, modifiers) {
            self.dispatch_action(NotoraAction::OpenSettings);
            return;
        }
        if let Some(action) = create_shortcut_action(key_code, modifiers) {
            self.dispatch_action(action);
            return;
        }
        if is_search_shortcut(key_code, modifiers) {
            self.dispatch_action(NotoraAction::FocusRequested(
                crate::FocusTarget::NavigationSearch,
            ));
            return;
        }
        if is_save_shortcut(key_code, modifiers) {
            self.request_manual_save();
            return;
        }
        let product_consumed = self.route_product_event(&key_event);
        if should_forward_input_to_editor(self.state().layout.focus_target, product_consumed) {
            self.handle_editor_key_input(key_code, modifiers);
        }
    }
}

fn should_forward_input_to_editor(focus_target: FocusTarget, product_consumed: bool) -> bool {
    focus_target == FocusTarget::Editor && !product_consumed
}

fn is_open_external_shortcut(key_code: ui::KeyCode, modifiers: Modifiers) -> bool {
    matches!(key_code, ui::KeyCode::Char('o') | ui::KeyCode::Char('O'))
        && (modifiers.cmd || modifiers.ctrl)
}

fn is_open_settings_shortcut(key_code: ui::KeyCode, modifiers: Modifiers) -> bool {
    matches!(key_code, ui::KeyCode::Char(',')) && (modifiers.cmd || modifiers.ctrl)
}

fn create_shortcut_action(key_code: ui::KeyCode, modifiers: Modifiers) -> Option<NotoraAction> {
    if !matches!(key_code, ui::KeyCode::Char('n') | ui::KeyCode::Char('N'))
        || !(modifiers.cmd || modifiers.ctrl)
    {
        return None;
    }
    Some(NotoraAction::OpenNewDocumentMenu)
}

fn is_save_shortcut(key_code: ui::KeyCode, modifiers: Modifiers) -> bool {
    matches!(key_code, ui::KeyCode::Char('s') | ui::KeyCode::Char('S'))
        && (modifiers.cmd || modifiers.ctrl)
}

fn is_search_shortcut(key_code: ui::KeyCode, modifiers: Modifiers) -> bool {
    matches!(key_code, ui::KeyCode::Char('f') | ui::KeyCode::Char('F'))
        && (modifiers.cmd || modifiers.ctrl)
}

fn map_mouse_button(button: WinitMouseButton) -> Option<ui::core::widget::MouseButton> {
    match button {
        WinitMouseButton::Left => Some(ui::core::widget::MouseButton::Left),
        WinitMouseButton::Right => Some(ui::core::widget::MouseButton::Right),
        WinitMouseButton::Middle => Some(ui::core::widget::MouseButton::Middle),
        WinitMouseButton::Back | WinitMouseButton::Forward | WinitMouseButton::Other(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use appkit_shell::canvas_viewport::CanvasViewportAction;
    use appkit_shell::editor_runtime::EditorFocus;
    use ui::core::Modifiers;
    use winit::dpi::PhysicalPosition;
    use winit::event::MouseScrollDelta;
    use winit::keyboard::ModifiersState;

    use super::{
        canvas_pinch_action, canvas_wheel_action, create_shortcut_action, editor_input_context,
        is_open_external_shortcut, is_open_settings_shortcut, is_save_shortcut, is_search_shortcut,
        should_forward_input_to_editor,
    };
    use crate::action::NotoraAction;
    use crate::{FocusTarget, NotoraState, OverlayState};

    #[test]
    fn only_an_active_editor_without_a_modal_can_receive_document_input() {
        let mut state = NotoraState::default();
        state.layout.focus_target = FocusTarget::Editor;
        let active_context = editor_input_context(&state, true);
        assert_eq!(active_context.focus, EditorFocus::Active);
        assert!(!active_context.modal_blocked);

        state.layout.overlay = OverlayState::Settings;
        let modal_context = editor_input_context(&state, true);
        assert!(modal_context.modal_blocked);

        state.layout.overlay = OverlayState::None;
        let unfocused_context = editor_input_context(&state, false);
        assert_eq!(unfocused_context.focus, EditorFocus::Inactive);
    }

    #[test]
    fn only_unconsumed_editor_input_is_forwarded_to_the_runtime() {
        for focus_target in [
            FocusTarget::NavigationSearch,
            FocusTarget::NavigationTree,
            FocusTarget::CardList,
            FocusTarget::EditorTitle,
            FocusTarget::EditorTag,
            FocusTarget::Overlay,
        ] {
            assert!(!should_forward_input_to_editor(focus_target, false));
        }
        assert!(!should_forward_input_to_editor(FocusTarget::Editor, true));
        assert!(should_forward_input_to_editor(FocusTarget::Editor, false));
    }

    #[test]
    fn command_or_control_o_uses_the_external_open_shortcut() {
        assert!(is_open_external_shortcut(
            ui::KeyCode::Char('o'),
            Modifiers { cmd: true, ..Modifiers::NONE }
        ));
        assert!(is_open_external_shortcut(
            ui::KeyCode::Char('O'),
            Modifiers { ctrl: true, ..Modifiers::NONE }
        ));
        assert!(!is_open_external_shortcut(ui::KeyCode::Char('o'), Modifiers::NONE));
    }

    #[test]
    fn command_or_control_comma_opens_settings() {
        assert!(is_open_settings_shortcut(
            ui::KeyCode::Char(','),
            Modifiers { cmd: true, ..Modifiers::NONE }
        ));
        assert!(is_open_settings_shortcut(
            ui::KeyCode::Char(','),
            Modifiers { ctrl: true, ..Modifiers::NONE }
        ));
        assert!(!is_open_settings_shortcut(ui::KeyCode::Char(','), Modifiers::NONE));
    }

    #[test]
    fn command_or_control_s_uses_the_explicit_save_shortcut() {
        assert!(is_save_shortcut(
            ui::KeyCode::Char('s'),
            Modifiers { cmd: true, ..Modifiers::NONE }
        ));
        assert!(is_save_shortcut(
            ui::KeyCode::Char('S'),
            Modifiers { ctrl: true, ..Modifiers::NONE }
        ));
        assert!(!is_save_shortcut(ui::KeyCode::Char('s'), Modifiers::NONE));
    }

    #[test]
    fn command_or_control_f_uses_the_global_search_shortcut() {
        assert!(is_search_shortcut(
            ui::KeyCode::Char('f'),
            Modifiers { cmd: true, ..Modifiers::NONE }
        ));
        assert!(is_search_shortcut(
            ui::KeyCode::Char('F'),
            Modifiers { ctrl: true, ..Modifiers::NONE }
        ));
        assert!(!is_search_shortcut(ui::KeyCode::Char('f'), Modifiers::NONE));
    }

    #[test]
    fn command_or_control_n_opens_the_new_document_menu_for_both_variants() {
        assert_eq!(
            create_shortcut_action(
                ui::KeyCode::Char('n'),
                Modifiers { cmd: true, ..Modifiers::NONE }
            ),
            Some(NotoraAction::OpenNewDocumentMenu)
        );
        assert_eq!(
            create_shortcut_action(
                ui::KeyCode::Char('N'),
                Modifiers { ctrl: true, shift: true, ..Modifiers::NONE }
            ),
            Some(NotoraAction::OpenNewDocumentMenu)
        );
        assert_eq!(create_shortcut_action(ui::KeyCode::Char('n'), Modifiers::NONE), None);
    }

    #[test]
    fn canvas_pixel_wheel_follows_natural_touchpad_on_both_axes() {
        let action = canvas_wheel_action(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(36.0, -72.0)),
            ModifiersState::empty(),
            (500.0, 400.0),
        );

        assert_eq!(
            action,
            Some(CanvasViewportAction::PanBy(ui::canvas::CanvasPoint::new(-36.0, 72.0)))
        );
    }

    #[test]
    fn canvas_command_wheel_and_pinch_zoom_at_the_pointer() {
        let anchor = (500.0, 400.0);
        let command_wheel = canvas_wheel_action(
            MouseScrollDelta::LineDelta(0.0, 0.25),
            ModifiersState::SUPER,
            anchor,
        );
        let pinch = canvas_pinch_action(0.2, anchor);

        let Some(CanvasViewportAction::ZoomBy {
            factor: wheel_factor,
            screen_anchor: wheel_anchor,
        }) = command_wheel
        else {
            panic!("command wheel should produce a zoom action");
        };
        let Some(CanvasViewportAction::ZoomBy {
            factor: pinch_factor,
            screen_anchor: pinch_anchor,
        }) = pinch
        else {
            panic!("pinch should produce a zoom action");
        };

        assert_eq!(wheel_anchor, ui::canvas::CanvasPoint::new(anchor.0, anchor.1));
        assert_eq!(pinch_anchor, wheel_anchor);
        assert!(wheel_factor > 1.0);
        assert!(pinch_factor > wheel_factor);
    }
}
