use crate::app::App;
use crate::app_effect::AppEffect;

pub(crate) enum TabScrollDirection {
    Left,
    Right,
}

pub(crate) enum ChromeDispatchAction {
    OpenPopup(ui::popup_menu::PopupMenu),
    ClearPopup,
    OpenOverflow,
    ScrollTab(TabScrollDirection),
    HoverTab(Option<usize>),
    SidebarResizeStart,
    SidebarResizeEnd,
    SetSidebarWidth(f32),
    ToggleSidebarPin,
    OpenSidebarSettingsMenu,
}

pub(crate) enum SettingsDispatchAction {
    SetViewMode(ui::view_mode::ViewMode),
    SetThemeMode(ui::settings::ThemeMode),
    SetFontFamily(String),
    SetFontSize(f32),
    SetLineHeightRatio(f32),
    SetTabWidth(usize),
    SetWordWrap(bool),
    SetShowLineNumbers(bool),
    SetShowStatusBar(bool),
    ToggleLineNumbers,
    ToggleWordWrap,
    ToggleStatusBar,
}

impl App {
    pub(crate) fn dispatch_chrome_action(&mut self, action: ChromeDispatchAction) -> AppEffect {
        match action {
            ChromeDispatchAction::OpenPopup(menu) => {
                let rect = menu.menu_rect;
                self.snapshot_popup_tab_ids();
                self.ui_shell
                    .push_overlay(Box::new(ui::popup_menu::PopupMenuWidget::new(menu)), rect);
                AppEffect::REDRAW
            }
            ChromeDispatchAction::ClearPopup => {
                self.clear_popup_tab_id_snapshot();
                self.ui_shell.clear_overlays();
                AppEffect::REDRAW
            }
            ChromeDispatchAction::OpenOverflow => {
                let screen = (self.screen_width(), self.screen_height());
                let metrics = self.ui_metrics();
                self.snapshot_popup_tab_ids();
                if let Some(layout) = self.ui_shell.tab_bar_layout() {
                    let entries = layout
                        .tabs
                        .iter()
                        .map(|entry| ui::popup_menu::OverflowEntry {
                            tab_index: entry.index,
                            title: entry.title.clone(),
                        })
                        .collect::<Vec<_>>();
                    if layout.dropdown_rect_px.w > 0.0 {
                        let menu = ui::popup_menu::PopupMenu::overflow_px(
                            &entries,
                            layout.dropdown_rect_px,
                            screen,
                            self.active_editor_index().unwrap_or(0),
                            metrics.dpi,
                        );
                        let rect = menu.menu_rect;
                        self.ui_shell.push_overlay(
                            Box::new(ui::popup_menu::PopupMenuWidget::new(menu)),
                            rect,
                        );
                    }
                }
                AppEffect::REDRAW
            }
            ChromeDispatchAction::ScrollTab(direction) => {
                if let Some(layout) = self.ui_shell.tab_bar_layout() {
                    let viewport_width = layout.clip_right_px - layout.clip_left_px;
                    let step = viewport_width * 0.7;
                    let delta = match direction {
                        TabScrollDirection::Left => -step,
                        TabScrollDirection::Right => step,
                    };
                    self.ui_shell.tab_bar_scroll_by(delta);
                }
                AppEffect::REDRAW
            }
            ChromeDispatchAction::HoverTab(index) => {
                self.ui_shell.set_tab_bar_hovered(index);
                AppEffect::NONE
            }
            ChromeDispatchAction::SetSidebarWidth(width) => {
                self.ui_shell.sidebar_cfg_mut().width = width;
                AppEffect::REDRAW
            }
            ChromeDispatchAction::SidebarResizeEnd => AppEffect::PERSIST_WORKSPACE,
            ChromeDispatchAction::SidebarResizeStart => AppEffect::NONE,
            ChromeDispatchAction::ToggleSidebarPin => {
                let pinned = !self.ui_shell.sidebar_pinned();
                self.ui_shell.set_sidebar_pinned(pinned);
                self.ui_shell.set_sidebar_visibility(if pinned {
                    ui::sidebar::Visibility::Pinned
                } else {
                    ui::sidebar::Visibility::Hidden
                });
                if !pinned {
                    self.ui_shell.set_sidebar_suppress_hover_enter(true);
                }
                AppEffect::REDRAW
            }
            ChromeDispatchAction::OpenSidebarSettingsMenu => {
                let screen_width = self.screen_width();
                let screen_height = self.screen_height();
                let button = self.ui_shell.sidebar_settings_button_rect();
                let button = (button.w > 0.0 && button.h > 0.0).then_some(button);
                let metrics = self.ui_metrics();
                let input = ui::sidebar::SidebarSettingsInput::from(&self.settings);
                let menu = ui::sidebar::build_settings_menu(
                    button,
                    &input,
                    screen_width,
                    screen_height,
                    &metrics,
                );
                self.ui_shell.sidebar_set_open_menu(menu);
                AppEffect::REDRAW
            }
        }
    }

    pub(crate) fn dispatch_settings_action(&mut self, action: SettingsDispatchAction) -> AppEffect {
        self.ui_shell.sidebar_set_open_menu(None);
        match action {
            SettingsDispatchAction::SetViewMode(mode) => {
                self.settings.set_view_mode(mode);
                self.ui_shell.sidebar_set_hovered(None);
                self.ui_shell.set_dragging_sidebar(false);
                AppEffect::PERSIST_SETTINGS.merge(AppEffect::SYNC_WINDOW_CHROME)
            }
            SettingsDispatchAction::SetThemeMode(mode) => {
                self.settings.set_theme_mode(mode);
                self.rebuild_theme_state();
                AppEffect::PERSIST_SETTINGS.merge(AppEffect::REDRAW)
            }
            SettingsDispatchAction::SetFontFamily(family) => {
                self.settings.set_font_family(family);
                AppEffect::PERSIST_SETTINGS.merge(AppEffect::RESHAPE)
            }
            SettingsDispatchAction::SetFontSize(size) => {
                self.settings.set_font_size(size);
                AppEffect::PERSIST_SETTINGS.merge(AppEffect::RESHAPE)
            }
            SettingsDispatchAction::SetLineHeightRatio(ratio) => {
                self.settings.set_line_height_ratio(ratio);
                AppEffect::PERSIST_SETTINGS.merge(AppEffect::RESHAPE)
            }
            SettingsDispatchAction::SetTabWidth(width) => {
                self.settings.set_tab_width(width);
                AppEffect::PERSIST_SETTINGS.merge(AppEffect::RESHAPE)
            }
            SettingsDispatchAction::SetWordWrap(enabled) => self.apply_word_wrap(enabled),
            SettingsDispatchAction::SetShowLineNumbers(enabled) => {
                self.settings.set_show_line_numbers(enabled);
                AppEffect::PERSIST_SETTINGS.merge(AppEffect::RESHAPE)
            }
            SettingsDispatchAction::SetShowStatusBar(enabled) => {
                self.settings.set_show_status_bar(enabled);
                AppEffect::PERSIST_SETTINGS.merge(AppEffect::RESHAPE)
            }
            SettingsDispatchAction::ToggleLineNumbers => self.dispatch_settings_action(
                SettingsDispatchAction::SetShowLineNumbers(!self.settings.show_line_numbers),
            ),
            SettingsDispatchAction::ToggleWordWrap => {
                self.apply_word_wrap(!self.settings.word_wrap)
            }
            SettingsDispatchAction::ToggleStatusBar => self.dispatch_settings_action(
                SettingsDispatchAction::SetShowStatusBar(!self.settings.show_status_bar),
            ),
        }
    }

    fn apply_word_wrap(&mut self, enabled: bool) -> AppEffect {
        self.settings.set_word_wrap(enabled);
        for tab_id in self.editor_tab_ids_in_order() {
            if let Some(mut tab) = self.tab_session_mut(tab_id) {
                tab.invalidate_render_cache_all();
            }
        }
        AppEffect::PERSIST_SETTINGS.merge(AppEffect::RESHAPE)
    }

    pub(crate) fn rebuild_theme_state(&mut self) {
        let system_theme = self
            .editor_runtime
            .window()
            .and_then(|window| window.theme())
            .unwrap_or(winit::window::Theme::Dark);
        self.current_theme = ui::Theme::resolve(
            self.settings.theme_mode,
            system_theme,
            &self.active_theme_pair,
            &self.theme_registry,
        );
        self.editor_runtime.update_theme(self.current_theme.clone());
    }

    pub(crate) fn handle_sidebar_key_action(
        &mut self,
        action: ui::sidebar::SidebarAction,
    ) -> AppEffect {
        use ui::sidebar::SidebarAction;
        let persistence = match action {
            SidebarAction::TogglePin | SidebarAction::PersistConfig => AppEffect::PERSIST_WORKSPACE,
            SidebarAction::StartResize
            | SidebarAction::ResizeTo(_)
            | SidebarAction::EndResize
            | SidebarAction::SetWidth(_)
            | SidebarAction::SwitchTab(_)
            | SidebarAction::CloseTab(_)
            | SidebarAction::NewDocument(_)
            | SidebarAction::OpenNewDocumentMenu
            | SidebarAction::OpenDocument
            | SidebarAction::OpenSettingsMenu
            | SidebarAction::SetViewMode(_)
            | SidebarAction::OpenSettingsFile
            | SidebarAction::ToggleViewMode
            | SidebarAction::Context { .. }
            | SidebarAction::Hovered
            | SidebarAction::ToggleLineNumbers
            | SidebarAction::ToggleWordWrap
            | SidebarAction::ToggleStatusBar
            | SidebarAction::SetThemeMode(_)
            | SidebarAction::ContextMenuPx { .. } => AppEffect::NONE,
        };
        persistence.merge(AppEffect::REDRAW)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_width_and_pin_return_redraw_without_applying() {
        let mut app = App::new(None);
        app.needs_redraw = false;

        let width_effect = app.dispatch_chrome_action(ChromeDispatchAction::SetSidebarWidth(260.0));
        let pin_effect = app.dispatch_chrome_action(ChromeDispatchAction::ToggleSidebarPin);

        assert_eq!(app.ui_shell.sidebar_width(), 260.0);
        assert!(width_effect.redraw);
        assert!(pin_effect.redraw);
        assert!(!app.needs_redraw);
    }

    #[test]
    fn sidebar_resize_end_requests_workspace_persistence() {
        let mut app = App::new(None);
        let effect = app.dispatch_chrome_action(ChromeDispatchAction::SidebarResizeEnd);
        assert!(effect.persist_workspace);
    }

    #[test]
    fn view_mode_returns_persist_chrome_and_redraw() {
        let mut app = App::new(None);
        app.needs_redraw = false;

        let effect = app.dispatch_settings_action(SettingsDispatchAction::SetViewMode(
            ui::view_mode::ViewMode::Tabs,
        ));

        assert_eq!(app.settings.view_mode, ui::view_mode::ViewMode::Tabs);
        assert!(effect.persist_settings);
        assert!(effect.sync_window_chrome);
        assert!(effect.redraw);
        assert!(!app.needs_redraw);
    }

    #[test]
    fn word_wrap_returns_persist_and_reshape() {
        let mut app = App::new(None);
        let before = app.settings.word_wrap;
        let effect = app.dispatch_settings_action(SettingsDispatchAction::ToggleWordWrap);

        assert_eq!(app.settings.word_wrap, !before);
        assert!(effect.persist_settings);
        assert!(effect.reshape);
    }
}
