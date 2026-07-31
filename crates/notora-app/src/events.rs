//! notora 产品事件与 editor runtime 输入门控。

use appkit_shell::ShellEvent;
use appkit_shell::editor_runtime::{EditorFocus, EditorInputContext};
use appkit_shell::window_input::{scroll_delta_pixels, ui_modifiers, winit_key_to_keycode};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton as WinitMouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::window::WindowId;

use crate::NotoraApp;
use crate::action::NotoraAction;
use crate::{FocusTarget, NotoraState, OverlayState, shell::layout::ShellLayout};
use ui::core::Modifiers;

/// 根据产品焦点和 overlay 状态构造 runtime 输入上下文。
pub fn editor_input_context(
    state: &NotoraState,
    layout: ShellLayout,
    window_focused: bool,
) -> EditorInputContext {
    let editor_is_focused = state.layout.focus_target == FocusTarget::Editor && window_focused;
    EditorInputContext {
        editor_rect: layout.editor_rect,
        focus: if editor_is_focused { EditorFocus::Active } else { EditorFocus::Inactive },
        modal_blocked: state.layout.overlay != OverlayState::None,
    }
}

impl ApplicationHandler<ShellEvent> for NotoraApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.resume(event_loop) {
            eprintln!("notora window initialization failed: {error}");
            event_loop.exit();
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: ShellEvent) {
        match event {
            ShellEvent::ProductWake => self.drain_product_events(),
            ShellEvent::StartBackgroundServices
            | ShellEvent::ReshapeResultsReady
            | ShellEvent::FileSafetyResultsReady
            | ShellEvent::SaveResultsReady => self.request_window_redraw(),
        }
    }

    fn window_event(
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
            WindowEvent::ModifiersChanged(modifiers) => {
                self.editor_runtime_mut().set_input_modifiers(modifiers.state());
            }
            WindowEvent::CursorMoved { position, .. } => {
                let (px, py) = (position.x as f32, position.y as f32);
                self.set_pointer_position(px, py);
                let product_consumed = self.route_product_event(&ui::Event::MouseMove { px, py });
                if !product_consumed {
                    let _ = self.runtime_accepts_pointer_input(px, py);
                }
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
                let product_consumed = self.route_product_event(&product_event);
                if product_consumed {
                    return;
                }
                match state {
                    ElementState::Pressed if self.runtime_accepts_pointer_input(px, py) => {
                        let _ = self.begin_editor_text_selection();
                    }
                    ElementState::Released => self.end_editor_pointer_capture(),
                    ElementState::Pressed => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (px, py) = self.pointer_position();
                let (dx, dy) = scroll_delta_pixels(&delta, self.shell_layout().dpi * 16.0);
                let product_consumed =
                    self.route_product_event(&ui::Event::Wheel { dx, dy, px, py });
                if !product_consumed {
                    let _ = self.runtime_accepts_pointer_input(px, py);
                }
            }
            WindowEvent::DroppedFile(path) => {
                self.receive_system_open_paths(vec![path]);
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                if !self.route_product_event(&ui::Event::ImePreedit { text: text.clone(), cursor })
                {
                    let _ = self.update_editor_preedit(text, cursor);
                }
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                if !self.route_product_event(&ui::Event::ImeCommit(text)) {
                    let _ = self.update_editor_preedit(String::new(), None);
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
                if key_code == ui::KeyCode::Escape {
                    self.dispatch_action(NotoraAction::EscapePressed);
                    return;
                }
                let modifiers = ui_modifiers(self.editor_runtime_mut().input_modifiers());
                if is_open_external_shortcut(key_code, modifiers) {
                    self.request_external_file_dialog();
                    return;
                }
                let key_event = ui::Event::KeyDown(key_code, modifiers);
                if !self.route_product_event(&key_event) {
                    let _ = self.runtime_accepts_keyboard_input();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.render() {
                    eprintln!("notora frame rendering failed: {error}");
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.take_redraw_request() {
            self.request_window_redraw();
        }
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

fn is_open_external_shortcut(key_code: ui::KeyCode, modifiers: Modifiers) -> bool {
    matches!(key_code, ui::KeyCode::Char('o') | ui::KeyCode::Char('O'))
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
    use appkit_shell::editor_runtime::EditorFocus;
    use ui::core::Modifiers;

    use super::{editor_input_context, is_open_external_shortcut};
    use crate::{FocusTarget, NotoraState, OverlayState, shell::layout::ShellLayoutInput};

    fn layout() -> crate::shell::layout::ShellLayout {
        crate::shell::layout::ShellLayout::compute(ShellLayoutInput {
            window_width_px: 1_200.0,
            window_height_px: 800.0,
            dpi: 1.0,
            navigation_width_logical: 220.0,
            card_list_width_logical: 340.0,
        })
    }

    #[test]
    fn only_an_active_editor_without_a_modal_can_receive_document_input() {
        let mut state = NotoraState::default();
        state.layout.focus_target = FocusTarget::Editor;
        let active_context = editor_input_context(&state, layout(), true);
        assert_eq!(active_context.focus, EditorFocus::Active);
        assert!(!active_context.modal_blocked);

        state.layout.overlay = OverlayState::Settings;
        let modal_context = editor_input_context(&state, layout(), true);
        assert!(modal_context.modal_blocked);

        state.layout.overlay = OverlayState::None;
        let unfocused_context = editor_input_context(&state, layout(), false);
        assert_eq!(unfocused_context.focus, EditorFocus::Inactive);
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
}
