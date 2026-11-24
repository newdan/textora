//! Routes settings and synchronization controls to their owners.

use crate::app::App;
use crate::app_effect::AppEffect;
use crate::dispatch::chrome::SettingsDispatchAction;
use crate::sync_settings_types::SyncSettingsAction;
use textora_sync::{DeviceId, LoopbackEndpoint, RemoteDeviceSpec, StaticSyncAddress};

const SYNC_INVALID_ENDPOINT_NOTICE: &str = "Syncthing 地址无效";
const SYNC_INVALID_API_KEY_NOTICE: &str = "API Key 无效";
const SYNC_INVALID_REMOTE_DEVICE_NOTICE: &str = "远端设备信息无效";
const SYNC_INVALID_REMOTE_ADDRESS_NOTICE: &str = "远端设备地址无效";
const SYNC_REMOTE_NAME_REQUIRED_NOTICE: &str = "远端设备名称不能为空";
const SYNC_LIBRARY_NOT_FOUND_NOTICE: &str = "资料库状态已更新，请重试";
const SYNC_PENDING_FOLDER_NOT_FOUND_NOTICE: &str = "待接收资料库不存在";

fn parse_sync_endpoint(candidate: &str) -> Result<LoopbackEndpoint, &'static str> {
    LoopbackEndpoint::parse(candidate.trim()).map_err(|_| SYNC_INVALID_ENDPOINT_NOTICE)
}

fn parse_sync_remote_device(
    device_id: String,
    name: String,
    raw_addresses: String,
) -> Result<RemoteDeviceSpec, &'static str> {
    let device_id = DeviceId::parse(device_id.trim().to_owned())
        .map_err(|_| SYNC_INVALID_REMOTE_DEVICE_NOTICE)?;
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(SYNC_REMOTE_NAME_REQUIRED_NOTICE);
    }

    let mut addresses = Vec::new();
    for address in raw_addresses.split(',').map(str::trim).filter(|value| !value.is_empty()) {
        if address == textora_sync::SYNCTHING_DYNAMIC_ADDRESS {
            continue;
        }
        addresses.push(
            StaticSyncAddress::parse(address.to_owned())
                .map_err(|_| SYNC_INVALID_REMOTE_ADDRESS_NOTICE)?,
        );
    }

    Ok(RemoteDeviceSpec { device_id, name, addresses })
}

fn sync_controller_error_notice(
    error: &crate::sync_controller::SyncControllerError,
) -> &'static str {
    match error {
        crate::sync_controller::SyncControllerError::InvalidApiKey => SYNC_INVALID_API_KEY_NOTICE,
        crate::sync_controller::SyncControllerError::WorkerUnavailable => "同步服务暂不可用",
    }
}

impl App {
    pub(crate) fn dispatch_settings_view_action(
        &mut self,
        action: ui::settings_view::SettingsViewAction,
    ) -> AppEffect {
        let effect = match action {
            ui::settings_view::SettingsViewAction::SetThemeMode(mode) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetThemeMode(mode))
            }
            ui::settings_view::SettingsViewAction::SetFontFamily(family) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetFontFamily(family))
            }
            ui::settings_view::SettingsViewAction::SetFontSize(size) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetFontSize(size))
            }
            ui::settings_view::SettingsViewAction::SetLineHeightRatio(ratio) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetLineHeightRatio(ratio))
            }
            ui::settings_view::SettingsViewAction::SetWordWrap(enabled) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetWordWrap(enabled))
            }
            ui::settings_view::SettingsViewAction::SetMarkdownFirstLineIndent(enabled) => self
                .dispatch_settings_action(SettingsDispatchAction::SetMarkdownFirstLineIndent(
                    enabled,
                )),
            ui::settings_view::SettingsViewAction::SetTextSpacingMode(mode) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetTextSpacingMode(mode))
            }
            ui::settings_view::SettingsViewAction::SetShowLineNumbers(enabled) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetShowLineNumbers(enabled))
            }
            ui::settings_view::SettingsViewAction::SetTabWidth(width) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetTabWidth(width))
            }
            ui::settings_view::SettingsViewAction::SetViewMode(mode) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetViewMode(mode))
            }
            ui::settings_view::SettingsViewAction::SetShowStatusBar(enabled) => {
                self.dispatch_settings_action(SettingsDispatchAction::SetShowStatusBar(enabled))
            }
            ui::settings_view::SettingsViewAction::RetryPersistence => AppEffect::PERSIST_SETTINGS,
        };
        self.refresh_settings_overlay();
        effect
    }

    pub(super) fn dispatch_sync_settings_action(
        &mut self,
        action: SyncSettingsAction,
    ) -> AppEffect {
        self.dispatch_sync_settings_action_with_folder_picker(action, &mut || {
            rfd::FileDialog::new().pick_folder()
        })
    }

    fn dispatch_sync_settings_action_with_folder_picker(
        &mut self,
        action: SyncSettingsAction,
        pick_folder: &mut dyn FnMut() -> Option<std::path::PathBuf>,
    ) -> AppEffect {
        match action {
            SyncSettingsAction::TestConnection { endpoint, api_key } => {
                let endpoint = match parse_sync_endpoint(&endpoint) {
                    Ok(endpoint) => endpoint,
                    Err(message) => return self.report_sync_settings_error(message),
                };
                let api_key = api_key.expose().to_owned();
                let result = self
                    .sync_controller_mut()
                    .map(|controller| controller.test_connection(endpoint, api_key))
                    .unwrap_or(Err(crate::sync_controller::SyncControllerError::WorkerUnavailable));
                self.finish_sync_controller_action(result)
            }
            SyncSettingsAction::ConfigureConnection { endpoint, api_key } => {
                let endpoint = match parse_sync_endpoint(&endpoint) {
                    Ok(endpoint) => endpoint,
                    Err(message) => return self.report_sync_settings_error(message),
                };
                let api_key = api_key.expose().to_owned();
                let result = self
                    .sync_controller_mut()
                    .map(|controller| controller.configure_connection(endpoint, api_key))
                    .unwrap_or(Err(crate::sync_controller::SyncControllerError::WorkerUnavailable));
                self.finish_sync_controller_action(result)
            }
            SyncSettingsAction::PublishLibrary {
                remote_device_id,
                remote_name,
                remote_addresses,
            } => {
                let Some(root) = pick_folder() else {
                    return AppEffect::NONE;
                };
                let remote = match parse_sync_remote_device(
                    remote_device_id,
                    remote_name,
                    remote_addresses.join(","),
                ) {
                    Ok(remote) => remote,
                    Err(message) => return self.report_sync_settings_error(message),
                };
                let result = self
                    .sync_controller_mut()
                    .map(|controller| controller.publish_library(root, remote))
                    .unwrap_or(Err(crate::sync_controller::SyncControllerError::WorkerUnavailable));
                self.finish_sync_controller_action(result)
            }
            SyncSettingsAction::AcceptRemoteLibrary { pending_index } => {
                let folder_id = self
                    .sync_controller()
                    .and_then(|controller| {
                        controller.snapshot().pending_folders.get(pending_index).cloned()
                    })
                    .map(|pending| pending.folder_id);
                let Some(folder_id) = folder_id else {
                    return self.report_sync_settings_error(SYNC_PENDING_FOLDER_NOT_FOUND_NOTICE);
                };
                let Some(empty_root) = pick_folder() else {
                    return AppEffect::NONE;
                };
                let result = self
                    .sync_controller_mut()
                    .map(|controller| controller.accept_remote_library(folder_id, empty_root))
                    .unwrap_or(Err(crate::sync_controller::SyncControllerError::WorkerUnavailable));
                self.finish_sync_controller_action(result)
            }
            SyncSettingsAction::ScanLibrary { library_index } => self
                .dispatch_library_controller_action(library_index, |controller, library_id| {
                    controller.scan_library(library_id)
                }),
            SyncSettingsAction::SetLibraryPaused { library_index, paused } => self
                .dispatch_library_controller_action(
                    library_index,
                    move |controller, library_id| controller.pause_library(library_id, paused),
                ),
            SyncSettingsAction::RepairLibrary { library_index } => self
                .dispatch_library_controller_action(library_index, |controller, library_id| {
                    controller.repair_library(library_id)
                }),
            SyncSettingsAction::RemoveLibraryMapping { library_index } => self
                .dispatch_library_controller_action(library_index, |controller, library_id| {
                    controller.remove_library_mapping(library_id)
                }),
            SyncSettingsAction::UnregisterLibrary { library_index } => self
                .dispatch_library_controller_action(library_index, |controller, library_id| {
                    controller.unregister_library(library_id)
                }),
        }
    }

    fn dispatch_library_controller_action<F>(
        &mut self,
        library_index: usize,
        operation: F,
    ) -> AppEffect
    where
        F: FnOnce(
            &mut crate::sync_controller::SyncController,
            String,
        ) -> Result<
            crate::sync_controller::RequestId,
            crate::sync_controller::SyncControllerError,
        >,
    {
        let Some(library_id) = self
            .sync_controller()
            .and_then(|controller| controller.snapshot().libraries.get(library_index))
            .map(|library| library.library_id.clone())
        else {
            return self.report_sync_settings_error(SYNC_LIBRARY_NOT_FOUND_NOTICE);
        };
        let result = self
            .sync_controller_mut()
            .map(|controller| operation(controller, library_id))
            .unwrap_or(Err(crate::sync_controller::SyncControllerError::WorkerUnavailable));
        self.finish_sync_controller_action(result)
    }

    fn finish_sync_controller_action(
        &mut self,
        result: Result<
            crate::sync_controller::RequestId,
            crate::sync_controller::SyncControllerError,
        >,
    ) -> AppEffect {
        match result {
            Ok(_) => AppEffect::REDRAW,
            Err(error) => self.report_sync_settings_error(sync_controller_error_notice(&error)),
        }
    }

    fn report_sync_settings_error(&mut self, message: &'static str) -> AppEffect {
        if let Some(controller) = self.sync_controller_mut() {
            controller.push_local_error(message.to_owned());
        } else {
            eprintln!("[sync] {message}");
        }
        AppEffect::REDRAW
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::AppAction;

    fn has_direct_sync_controller_field_access(compact_source: &str) -> bool {
        let sync_controller_field = ["self.sync_", "controller"].concat();

        compact_source.match_indices(&sync_controller_field).any(|(index, _)| {
            let suffix = &compact_source[index + sync_controller_field.len()..];
            !suffix.starts_with("()") && !suffix.starts_with("_mut()")
        })
    }

    #[test]
    fn dispatch_routes_sync_controller_access_through_app_accessors() {
        let production_source = include_str!("settings.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("dispatch production source should precede tests");
        let compact_production_source = production_source.split_whitespace().collect::<String>();

        assert!(
            !has_direct_sync_controller_field_access(&compact_production_source),
            "dispatch must use App sync-controller accessors"
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
    fn settings_actions_apply_values_and_return_required_effects() {
        let mut app = App::new(None);

        assert!(
            app.dispatch_settings_view_action(ui::settings_view::SettingsViewAction::SetFontSize(
                18.0,
            ))
            .reshape
        );
        assert_eq!(app.editor_runtime.settings().font_size, 18.0);

        assert!(
            app.dispatch_settings_view_action(ui::settings_view::SettingsViewAction::SetThemeMode(
                ui::settings::ThemeMode::Light,
            ),)
                .redraw
        );
        assert_eq!(app.editor_runtime.settings().theme_mode, ui::settings::ThemeMode::Light);

        assert!(
            app.dispatch_settings_view_action(ui::settings_view::SettingsViewAction::SetViewMode(
                ui::view_mode::ViewMode::Tabs,
            ))
            .sync_window_chrome
        );
        assert_eq!(app.editor_runtime.settings().view_mode, ui::view_mode::ViewMode::Tabs);
    }

    #[test]
    fn sync_settings_action_reaches_existing_controller_validation() {
        let mut app = App::new(None);
        let sync_action = SyncSettingsAction::TestConnection {
            endpoint: "https://example.com".to_owned(),
            api_key: ui::core::widget::SensitiveText::new("secret".to_owned()),
        };
        let debug = format!("{sync_action:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret"));

        let effect = app.reduce_action(AppAction::Sync(sync_action), None);
        assert!(effect.redraw);
    }

    #[test]
    fn publish_library_prompts_for_folder_before_validating_remote_fields() {
        let mut app = App::new(None);
        let mut folder_picker_call_count = 0;

        let effect = app.dispatch_sync_settings_action_with_folder_picker(
            SyncSettingsAction::PublishLibrary {
                remote_device_id: String::new(),
                remote_name: String::new(),
                remote_addresses: Vec::new(),
            },
            &mut || {
                folder_picker_call_count += 1;
                None
            },
        );

        assert_eq!(folder_picker_call_count, 1);
        assert_eq!(effect, AppEffect::NONE);
    }

    #[test]
    fn sidebar_settings_action_opens_modal_settings_view() {
        let mut app = App::new(None);

        let effect = app.reduce_action(AppAction::OpenSidebarSettingsMenu, None);

        assert!(effect.redraw);
        assert_eq!(app.ui_shell.overlays_count(), 1);
        assert!(app.ui_shell.active_overlay_is_modal());
    }

    #[test]
    fn sync_remote_input_parser_accepts_static_addresses_and_dynamic_placeholder() {
        let remote = parse_sync_remote_device(
            "ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG".to_owned(),
            "远端设备".to_owned(),
            " tcp://sync.example.com:22000, dynamic ".to_owned(),
        )
        .expect("valid sync remote input should parse");

        assert_eq!(
            remote.device_id.as_str(),
            "ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG"
        );
        assert_eq!(remote.name, "远端设备");
        assert_eq!(remote.addresses.len(), 1);
        assert_eq!(remote.addresses[0].as_str(), "tcp://sync.example.com:22000");
    }

    #[test]
    fn sync_remote_input_parser_rejects_invalid_device_or_address() {
        assert!(
            parse_sync_remote_device(
                "invalid".to_owned(),
                "远端设备".to_owned(),
                "dynamic".to_owned(),
            )
            .is_err()
        );
        assert!(
            parse_sync_remote_device(
                "ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG-ABCDEFG".to_owned(),
                "远端设备".to_owned(),
                "https://sync.example.com".to_owned(),
            )
            .is_err()
        );
    }
}
