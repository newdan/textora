use super::*;
use ui::MouseButton;
use ui::core::measure::NoopMeasure;
use ui::core::paint::{DrawCmd, DrawList};
use ui::core::{KeyCode, Modifiers};

fn overlay_for_window(width: f32, height: f32, dpi: f32) -> SettingsOverlay {
    let theme = ui::theme::test_theme();
    let mut measure = NoopMeasure;
    let mut overlay = SettingsOverlay::new();
    overlay.set_rect(
        Rect::new(0.0, 0.0, width * dpi, height * dpi),
        &mut LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi },
    );
    overlay
}

fn rounded_rect_contains(rect: Rect, radius: f32, x: f32, y: f32) -> bool {
    if !rect.contains(x, y) {
        return false;
    }
    let radius = radius.min(rect.w * 0.5).min(rect.h * 0.5);
    let center_x = x.clamp(rect.x + radius, rect.right() - radius);
    let center_y = y.clamp(rect.y + radius, rect.bottom() - radius);
    (x - center_x).powi(2) + (y - center_y).powi(2) <= radius.powi(2)
}

fn fill_alpha_at(commands: &[DrawCmd], x: f32, y: f32) -> f32 {
    let mut clips = Vec::new();
    let mut alpha = 0.0;
    for command in commands {
        match command {
            DrawCmd::PushClip(rect) => clips.push(*rect),
            DrawCmd::PopClip => {
                clips.pop();
            }
            DrawCmd::FillRect { rect, color, radius }
                if clips.iter().all(|clip| clip.contains(x, y))
                    && rounded_rect_contains(*rect, *radius, x, y) =>
            {
                alpha = color[3] + alpha * (1.0 - color[3]);
            }
            _ => {}
        }
    }
    alpha
}

#[test]
fn all_panel_corners_remain_transparent_outside_the_rounded_outline() {
    for dpi in [1.0, 2.0] {
        let overlay = overlay_for_window(1_200.0, 800.0, dpi);
        let theme = ui::theme::test_theme();
        let mut draws = DrawList::new();
        overlay.paint(&mut PaintCtx::new(&mut draws, &theme, dpi));
        let panel = overlay.panel_rect();
        for x in [panel.x + dpi, panel.right() - dpi] {
            for y in [panel.y + dpi, panel.bottom() - dpi] {
                assert_eq!(fill_alpha_at(&draws.cmds, x, y), 0.0, "corner at {x}, {y}");
            }
            assert_eq!(fill_alpha_at(&draws.cmds, x, panel.y + panel.h * 0.5), 1.0);
        }
    }
}

#[test]
fn corner_backgrounds_do_not_stack_antialiased_edges() {
    let overlay = overlay_for_window(1_200.0, 800.0, 1.0);
    let theme = ui::theme::test_theme();
    let mut draws = DrawList::new();
    overlay.paint(&mut PaintCtx::new(&mut draws, &theme, 1.0));
    // Half opacity makes any overlapping background layers observable in the composition.
    for command in &mut draws.cmds {
        if let DrawCmd::FillRect { color, .. } = command {
            color[3] = 0.5;
        }
    }
    let panel = overlay.panel_rect();
    let inset = PANEL_CORNER_RADIUS_LOGICAL * 0.5;
    for x in [panel.x + inset, panel.right() - inset] {
        for y in [panel.y + inset, panel.bottom() - inset] {
            assert_eq!(fill_alpha_at(&draws.cmds, x, y), 0.5);
        }
    }
}

fn close_center(overlay: &SettingsOverlay, dpi: f32) -> (f32, f32) {
    let panel = overlay.panel_rect();
    (panel.right() - 28.0 * dpi, panel.y + 28.0 * dpi)
}

#[test]
fn top_right_close_button_dismisses_at_regular_and_compact_sizes() {
    for (width, height) in [(1_200.0, 800.0), (500.0, 400.0)] {
        for dpi in [1.0, 2.0] {
            let mut overlay = overlay_for_window(width, height, dpi);
            let (px, py) = close_center(&overlay, dpi);
            let theme = ui::theme::test_theme();
            let mut context = EventCtx::new(&theme, dpi);
            assert_ne!(
                overlay.route_event(
                    &Event::MouseDown { px, py, button: MouseButton::Left },
                    &mut context,
                ),
                Some(SettingsOverlayAction::Dismiss)
            );
            assert_eq!(
                overlay.route_event(
                    &Event::MouseUp { px, py, button: MouseButton::Left },
                    &mut context,
                ),
                Some(SettingsOverlayAction::Dismiss)
            );
        }
    }
}

#[test]
fn escape_dismisses_settings() {
    let mut overlay = overlay_for_window(1_200.0, 800.0, 1.0);
    let theme = ui::theme::test_theme();
    assert_eq!(
        overlay.route_event(
            &Event::KeyDown(KeyCode::Escape, Modifiers::NONE),
            &mut EventCtx::new(&theme, 1.0),
        ),
        Some(SettingsOverlayAction::Dismiss)
    );
}

#[test]
fn releasing_outside_or_cancelling_does_not_activate_close() {
    let theme = ui::theme::test_theme();
    for cancel in [false, true] {
        let mut overlay = overlay_for_window(1_200.0, 800.0, 1.0);
        let (px, py) = close_center(&overlay, 1.0);
        let mut context = EventCtx::new(&theme, 1.0);
        overlay.route_event(&Event::MouseDown { px, py, button: MouseButton::Left }, &mut context);
        if cancel {
            overlay.route_event(&Event::InteractionCancel, &mut context);
        } else {
            overlay.route_event(&Event::MouseMove { px: 0.0, py: 0.0 }, &mut context);
            assert_ne!(
                overlay.route_event(
                    &Event::MouseUp { px: 0.0, py: 0.0, button: MouseButton::Left },
                    &mut context,
                ),
                Some(SettingsOverlayAction::Dismiss)
            );
        }
        assert_ne!(
            overlay
                .route_event(&Event::MouseUp { px, py, button: MouseButton::Left }, &mut context,),
            Some(SettingsOverlayAction::Dismiss)
        );
    }
}

#[test]
fn close_icon_is_visible_and_section_text_stays_below_header() {
    for theme in [ui::theme::test_theme(), ui::theme::test_light_theme()] {
        let mut overlay = overlay_for_window(500.0, 400.0, 2.0);
        let mut measure = NoopMeasure;
        overlay.set_rect(
            Rect::new(0.0, 0.0, 1_000.0, 800.0),
            &mut LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 2.0 },
        );
        let mut draws = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("settings visual test needs fonts");
        overlay.paint(&mut PaintCtx {
            list: &mut draws,
            theme: &theme,
            dpi: 2.0,
            offset: (0.0, 0.0),
            global_alpha: 1.0,
            shaper: Some(&mut shaper),
        });
        let button = overlay.close_button.rect();
        assert!(draws.cmds.iter().any(|command| matches!(command,
            DrawCmd::FillTriangle { p0, p1, p2, color }
                if color[3] > 0.0 && [p0, p1, p2].iter().all(|point| button.contains(point[0], point[1]))
        )));
        let header_bottom = overlay.panel_rect().y + PANEL_HEADER_HEIGHT_LOGICAL * 2.0;
        for command in &draws.cmds {
            if let DrawCmd::TextLayout { layout, y_baseline, .. } = command
                && layout.text != "设置"
            {
                assert!(
                    *y_baseline - layout.font_size >= header_bottom,
                    "{} overlaps the header",
                    layout.text
                );
            }
        }
    }
}

fn painted_border_positions(overlay: &SettingsOverlay) -> Vec<Rect> {
    let theme = ui::theme::test_theme();
    let mut draws = DrawList::new();
    overlay.paint(&mut PaintCtx::new(&mut draws, &theme, 1.0));
    draws
        .cmds
        .into_iter()
        .filter_map(|command| match command {
            DrawCmd::StrokeRect { rect, .. } => Some(rect),
            _ => None,
        })
        .collect()
}

#[test]
fn escape_during_scrollbar_drag_does_not_resume_drag_after_reopening() {
    let mut overlay = overlay_for_window(500.0, 400.0, 1.0);
    let theme = ui::theme::test_theme();
    let mut context = EventCtx::new(&theme, 1.0);
    let panel = overlay.panel_rect();
    let category_x = panel.x + 30.0;
    let category_y = panel.y + PANEL_HEADER_HEIGHT_LOGICAL + 12.0 + 38.0 + 17.0;
    for event in [
        Event::MouseDown { px: category_x, py: category_y, button: MouseButton::Left },
        Event::MouseUp { px: category_x, py: category_y, button: MouseButton::Left },
    ] {
        overlay.route_event(&event, &mut context);
    }
    let mut measure = NoopMeasure;
    let mut layout = LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    overlay.set_rect(Rect::new(0.0, 0.0, 500.0, 400.0), &mut layout);
    let before_drag = painted_border_positions(&overlay);
    let px = panel.right() - 14.0;
    let py = panel.y + 80.0;
    overlay.route_event(&Event::MouseDown { px, py, button: MouseButton::Left }, &mut context);
    overlay.route_event(&Event::MouseMove { px, py: py + 40.0 }, &mut context);
    assert_ne!(
        painted_border_positions(&overlay),
        before_drag,
        "precondition: scrollbar must be dragging"
    );
    assert_eq!(
        overlay.route_event(&Event::KeyDown(KeyCode::Escape, Modifiers::NONE), &mut context,),
        Some(SettingsOverlayAction::Dismiss)
    );
    overlay.set_rect(Rect::new(0.0, 0.0, 500.0, 400.0), &mut layout);
    let after_reopening = painted_border_positions(&overlay);
    overlay.route_event(&Event::MouseMove { px, py: py + 80.0 }, &mut context);
    assert_eq!(painted_border_positions(&overlay), after_reopening);
}
