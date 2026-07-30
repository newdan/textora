use crate::app::App;
use crate::app::SettingsPersistenceState;
use crate::app_effect::AppEffect;

const SETTINGS_OVERLAY_PREFERRED_WIDTH_LOGICAL: f32 = 720.0;
const SETTINGS_OVERLAY_PREFERRED_HEIGHT_LOGICAL: f32 = 560.0;
const SETTINGS_OVERLAY_MIN_MARGIN_LOGICAL: f32 = 24.0;
const SETTINGS_OVERLAY_MAX_WIDTH_RATIO: f32 = 0.92;
const SETTINGS_OVERLAY_MAX_HEIGHT_RATIO: f32 = 0.90;

fn settings_overlay_layout() -> ui::OverlayLayout {
    ui::OverlayLayout::Centered {
        preferred_size: (
            SETTINGS_OVERLAY_PREFERRED_WIDTH_LOGICAL,
            SETTINGS_OVERLAY_PREFERRED_HEIGHT_LOGICAL,
        ),
        min_margin: SETTINGS_OVERLAY_MIN_MARGIN_LOGICAL,
        max_width_ratio: SETTINGS_OVERLAY_MAX_WIDTH_RATIO,
        max_height_ratio: SETTINGS_OVERLAY_MAX_HEIGHT_RATIO,
    }
}

impl SettingsPersistenceState {
    pub(crate) fn to_view(&self) -> ui::settings_view::SettingsPersistenceView {
        match self {
            Self::Saved => ui::settings_view::SettingsPersistenceView::Saved,
            Self::SaveFailed { message } => {
                ui::settings_view::SettingsPersistenceView::SaveFailed { message: message.clone() }
            }
        }
    }
}

impl App {
    pub(crate) fn settings_view_input(&self) -> ui::settings_view::SettingsViewInput {
        ui::settings_view::SettingsViewInput {
            theme_mode: self.settings.theme_mode,
            font_family: self.settings.font_family.clone(),
            font_size: self.settings.font_size,
            line_height_ratio: self.settings.line_height_ratio,
            word_wrap: self.settings.word_wrap,
            show_line_numbers: self.settings.show_line_numbers,
            tab_width: self.settings.tab_width,
            view_mode: self.settings.view_mode,
            show_status_bar: self.settings.show_status_bar,
            persistence: self.settings_persistence.to_view(),
        }
    }

    pub(crate) fn open_settings_overlay(&mut self) -> AppEffect {
        let sync_input = self
            .sync_controller_mut()
            .map(|controller| {
                let notices = controller.drain_notices();
                crate::sync_view_model::build_sync_settings_input(controller.snapshot(), &notices)
            })
            .unwrap_or_else(crate::sync_view_model::empty_sync_settings_input);
        let overlay = crate::textora_settings_overlay::TextoraSettingsOverlay::new(
            self.settings_view_input(),
            sync_input,
        );
        let frame = ui::modal_frame::ModalFrame::new("设置", Box::new(overlay));
        self.ui_shell.clear_overlays();
        self.ui_shell.push_overlay_with_policy(
            Box::new(frame),
            settings_overlay_layout(),
            ui::OverlayInputPolicy::Modal,
            ui::DismissPolicy::EscapeOrExplicit,
        );
        AppEffect::REDRAW
    }

    pub(crate) fn take_pending_sync_settings_action(
        &mut self,
    ) -> Option<crate::sync_settings_types::SyncSettingsAction> {
        self.ui_shell
            .active_overlay_widget_mut::<ui::modal_frame::ModalFrame>()?
            .content_as_any_mut()
            .downcast_mut::<crate::textora_settings_overlay::TextoraSettingsOverlay>()?
            .take_pending_sync_action()
    }

    pub(crate) fn refresh_settings_overlay(&mut self) {
        let input = self.settings_view_input();
        let Some(frame) = self.ui_shell.active_overlay_widget_mut::<ui::modal_frame::ModalFrame>()
        else {
            return;
        };
        let Some(overlay) = frame
            .content_as_any_mut()
            .downcast_mut::<crate::textora_settings_overlay::TextoraSettingsOverlay>(
        ) else {
            return;
        };
        overlay.set_settings_input(input);
    }

    pub(crate) fn refresh_sync_settings_overlay(&mut self) {
        let settings_overlay_is_active = self
            .ui_shell
            .active_overlay_widget_mut::<ui::modal_frame::ModalFrame>()
            .is_some_and(|frame| {
                frame
                    .content_as_any_mut()
                    .is::<crate::textora_settings_overlay::TextoraSettingsOverlay>()
            });
        if !settings_overlay_is_active {
            return;
        }

        let Some(sync_input) = self.sync_controller_mut().map(|controller| {
            let notices = controller.drain_notices();
            crate::sync_view_model::build_sync_settings_input(controller.snapshot(), &notices)
        }) else {
            return;
        };

        let Some(frame) = self.ui_shell.active_overlay_widget_mut::<ui::modal_frame::ModalFrame>()
        else {
            return;
        };
        let Some(overlay) = frame
            .content_as_any_mut()
            .downcast_mut::<crate::textora_settings_overlay::TextoraSettingsOverlay>(
        ) else {
            return;
        };
        overlay.set_sync_input(sync_input);
    }

    pub(crate) fn record_settings_persistence_result(&mut self, result: std::io::Result<()>) {
        let failed = result.is_err();
        self.settings_persistence = match result {
            Ok(()) => SettingsPersistenceState::Saved,
            Err(error) => SettingsPersistenceState::SaveFailed { message: error.to_string() },
        };
        self.refresh_settings_overlay();
        if failed {
            self.needs_redraw = true;
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_direct_sync_controller_field_access(compact_source: &str) -> bool {
        let sync_controller_field = ["self.sync_", "controller"].concat();

        compact_source.match_indices(&sync_controller_field).any(|(index, _)| {
            let suffix = &compact_source[index + sync_controller_field.len()..];
            !suffix.starts_with("()") && !suffix.starts_with("_mut()")
        })
    }

    #[test]
    fn settings_overlay_routes_sync_controller_access_through_app_accessors() {
        let production_source = include_str!("settings_overlay.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("settings overlay production source should precede tests");
        let compact_production_source = production_source.split_whitespace().collect::<String>();

        assert!(
            !has_direct_sync_controller_field_access(&compact_production_source),
            "settings overlay must use App sync-controller accessors"
        );
    }

    #[test]
    fn sync_controller_boundary_rejects_fields_without_rejecting_accessors() {
        let sync_controller_field = ["self.sync_", "controller"].concat();

        assert!(has_direct_sync_controller_field_access(&format!(
            "{sync_controller_field}.as_ref()"
        )));
        assert!(has_direct_sync_controller_field_access(&format!("{sync_controller_field}=None;")));
        assert!(!has_direct_sync_controller_field_access(&format!("{sync_controller_field}()")));
        assert!(!has_direct_sync_controller_field_access(&format!(
            "{sync_controller_field}_mut()"
        )));
    }

    #[test]
    fn settings_overlay_uses_expanded_preferred_height() {
        assert_eq!(
            settings_overlay_layout().resolve(ui::Rect::new(0.0, 0.0, 1200.0, 800.0), 1.0),
            ui::Rect::new(240.0, 120.0, 720.0, 560.0),
        );
    }

    #[test]
    fn pending_sync_action_requires_an_active_textora_settings_modal() {
        let mut app = App::new(None);
        assert_eq!(app.take_pending_sync_settings_action(), None);

        let generic_settings_input = app.settings_view_input();
        app.ui_shell.push_overlay_with_policy(
            Box::new(ui::modal_frame::ModalFrame::new(
                "设置",
                Box::new(ui::settings_view::SettingsView::new(generic_settings_input)),
            )),
            ui::OverlayLayout::Fixed(ui::Rect::new(0.0, 0.0, 720.0, 560.0)),
            ui::OverlayInputPolicy::Modal,
            ui::DismissPolicy::ExplicitOnly,
        );

        assert_eq!(app.take_pending_sync_settings_action(), None);
    }

    #[test]
    fn opening_preferences_creates_one_modal_settings_overlay() {
        let mut app = App::new(None);
        app.open_settings_overlay();
        app.open_settings_overlay();

        assert_eq!(app.ui_shell.overlays_count(), 1);
        assert!(app.ui_shell.active_overlay_is_modal());
        let frame = app
            .ui_shell
            .active_overlay_widget_mut::<ui::modal_frame::ModalFrame>()
            .expect("settings overlay must use ModalFrame");
        assert!(
            frame
                .content_as_any_mut()
                .downcast_mut::<crate::textora_settings_overlay::TextoraSettingsOverlay>()
                .is_some()
        );
    }

    #[test]
    fn failed_persistence_keeps_runtime_value_and_exposes_retry() {
        let mut app = App::new(None);
        app.dispatch_settings_view_action(ui::settings_view::SettingsViewAction::SetFontSize(20.0));
        app.record_settings_persistence_result(Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "permission denied",
        )));

        assert_eq!(app.settings.font_size, 20.0);
        assert!(matches!(app.settings_persistence, SettingsPersistenceState::SaveFailed { .. }));
        assert!(matches!(
            app.settings_view_input().persistence,
            ui::settings_view::SettingsPersistenceView::SaveFailed { .. }
        ));
    }
}
