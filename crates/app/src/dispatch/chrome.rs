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
}

pub(crate) enum SettingsDispatchAction {
    SetViewMode(ui::view_mode::ViewMode),
    SetThemeMode(ui::settings::ThemeMode),
    SetFontFamily(String),
    SetFontSize(f32),
    SetLineHeightRatio(f32),
    SetTabWidth(usize),
    SetWordWrap(bool),
    SetMarkdownFirstLineIndent(bool),
    SetTextSpacingMode(ui::typography::TextSpacingMode),
    SetShowLineNumbers(bool),
    SetShowStatusBar(bool),
    ToggleLineNumbers,
    ToggleWordWrap,
    ToggleStatusBar,
}

impl SettingsDispatchAction {
    fn apply_to(self, settings: &mut ui::Settings) {
        match self {
            Self::SetViewMode(mode) => settings.set_view_mode(mode),
            Self::SetThemeMode(mode) => settings.set_theme_mode(mode),
            Self::SetFontFamily(family) => settings.set_font_family(family),
            Self::SetFontSize(size) => settings.set_font_size(size),
            Self::SetLineHeightRatio(ratio) => settings.set_line_height_ratio(ratio),
            Self::SetTabWidth(width) => settings.set_tab_width(width),
            Self::SetWordWrap(enabled) => settings.set_word_wrap(enabled),
            Self::SetMarkdownFirstLineIndent(enabled) => {
                settings.set_markdown_first_line_indent(enabled);
            }
            Self::SetTextSpacingMode(mode) => settings.set_text_spacing_mode(mode),
            Self::SetShowLineNumbers(enabled) => settings.set_show_line_numbers(enabled),
            Self::SetShowStatusBar(enabled) => settings.set_show_status_bar(enabled),
            Self::ToggleLineNumbers => settings.set_show_line_numbers(!settings.show_line_numbers),
            Self::ToggleWordWrap => settings.set_word_wrap(!settings.word_wrap),
            Self::ToggleStatusBar => settings.set_show_status_bar(!settings.show_status_bar),
        }
    }
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
                if let Some((layout, bounds)) = self.ui_shell.tab_bar_layout_and_bounds() {
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
                            ui::core::Rect::new(
                                bounds.x + layout.dropdown_rect_px.x,
                                bounds.y + layout.dropdown_rect_px.y,
                                layout.dropdown_rect_px.w,
                                layout.dropdown_rect_px.h,
                            ),
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
                let dpi = self.ui_metrics().dpi.max(f32::EPSILON);
                self.ui_shell.set_sidebar_width_logical(width / dpi);
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
        }
    }

    pub(crate) fn dispatch_settings_action(&mut self, action: SettingsDispatchAction) -> AppEffect {
        let menu_closed = self.ui_shell.sidebar_clear_menu();
        let previous = self.editor_runtime.settings();
        let (view_mode, theme_mode, word_wrap) =
            (previous.view_mode, previous.theme_mode, previous.word_wrap);
        if !self.editor_runtime.edit_settings(|settings| action.apply_to(settings)) {
            return if menu_closed { AppEffect::REDRAW } else { AppEffect::NONE };
        }

        let current = self.editor_runtime.settings();
        let effect = if current.view_mode != view_mode {
            self.ui_shell.set_dragging_sidebar(false);
            AppEffect::SYNC_WINDOW_CHROME
        } else if current.theme_mode != theme_mode {
            let system_theme = self
                .editor_runtime
                .window()
                .and_then(|window| window.theme())
                .unwrap_or(winit::window::Theme::Dark);
            self.rebuild_theme_state(system_theme);
            AppEffect::REDRAW
        } else {
            if current.word_wrap != word_wrap {
                self.invalidate_tab_render_caches();
            }
            AppEffect::RESHAPE
        };
        effect.merge(AppEffect::PERSIST_SETTINGS)
    }

    fn invalidate_tab_render_caches(&mut self) {
        for tab_id in self.editor_tab_ids_in_order() {
            if let Some(mut tab) = self.tab_session_mut(tab_id) {
                tab.invalidate_render_cache_all();
            }
        }
    }

    pub(crate) fn rebuild_theme_state(&mut self, system_theme: winit::window::Theme) {
        let theme =
            self.settings_ui.resolve_theme(self.editor_runtime.settings().theme_mode, system_theme);
        self.editor_runtime.update_theme(theme);
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
        app.platform.clear_redraw();

        let width_effect = app.dispatch_chrome_action(ChromeDispatchAction::SetSidebarWidth(260.0));
        let pin_effect = app.dispatch_chrome_action(ChromeDispatchAction::ToggleSidebarPin);

        assert_eq!(app.ui_shell.sidebar_width_logical(), 260.0);
        assert!(width_effect.redraw);
        assert!(pin_effect.redraw);
        assert!(!app.platform.redraw_pending());
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
        app.platform.clear_redraw();

        let effect = app.dispatch_settings_action(SettingsDispatchAction::SetViewMode(
            ui::view_mode::ViewMode::Tabs,
        ));

        assert_eq!(app.editor_runtime.settings().view_mode, ui::view_mode::ViewMode::Tabs);
        assert!(effect.persist_settings);
        assert!(effect.sync_window_chrome);
        assert!(effect.redraw);
        assert!(!app.platform.redraw_pending());
    }

    #[test]
    fn word_wrap_returns_persist_and_reshape() {
        let mut app = App::new(None);
        let before = app.editor_runtime.settings().word_wrap;
        let effect = app.dispatch_settings_action(SettingsDispatchAction::ToggleWordWrap);

        assert_eq!(app.editor_runtime.settings().word_wrap, !before);
        assert!(effect.persist_settings);
        assert!(effect.reshape);
    }

    #[test]
    fn unchanged_settings_do_not_request_persistence_or_invalidate_layout() {
        let mut app = App::new(None);
        let original_version = app.editor_runtime.settings().version;
        let unchanged =
            SettingsDispatchAction::SetWordWrap(app.editor_runtime.settings().word_wrap);

        let effect = app.dispatch_settings_action(unchanged);

        assert_eq!(effect, AppEffect::NONE);
        assert_eq!(app.editor_runtime.settings().version, original_version);
    }

    #[test]
    fn settings_view_reflects_commits_to_the_editor_runtime() {
        use ui::settings::ThemeMode;
        use ui::settings_view::SettingsViewInput;
        use ui::typography::TextSpacingMode;
        use ui::view_mode::ViewMode;

        let mut app = App::new(None);
        let before = app.settings_view_input();
        let original_version = app.editor_runtime.settings().version;
        let actions = [
            SettingsDispatchAction::SetViewMode(ViewMode::Tabs),
            SettingsDispatchAction::SetThemeMode(ThemeMode::Light),
            SettingsDispatchAction::SetFontFamily("Helvetica".into()),
            SettingsDispatchAction::SetFontSize(21.0),
            SettingsDispatchAction::SetLineHeightRatio(1.75),
            SettingsDispatchAction::SetTabWidth(8),
            SettingsDispatchAction::ToggleWordWrap,
            SettingsDispatchAction::SetMarkdownFirstLineIndent(true),
            SettingsDispatchAction::SetTextSpacingMode(TextSpacingMode::Verbatim),
            SettingsDispatchAction::ToggleLineNumbers,
            SettingsDispatchAction::ToggleStatusBar,
        ];
        let revisions = actions.len() as u64;
        for action in actions {
            assert!(app.dispatch_settings_action(action).persist_settings);
        }
        assert_eq!(app.editor_runtime.settings().version, original_version + revisions);
        assert!(!app.editor_runtime.theme().is_dark);
        assert_eq!(
            app.settings_view_input(),
            SettingsViewInput {
                theme_mode: ThemeMode::Light,
                font_family: "Helvetica".into(),
                font_size: 21.0,
                line_height_ratio: 1.75,
                word_wrap: !before.word_wrap,
                markdown_first_line_indent: true,
                text_spacing_mode: TextSpacingMode::Verbatim,
                show_line_numbers: !before.show_line_numbers,
                tab_width: 8,
                view_mode: ViewMode::Tabs,
                show_status_bar: !before.show_status_bar,
                persistence: before.persistence,
            },
        );
    }

    #[test]
    fn text_spacing_mode_returns_persist_and_reshape() {
        let mut app = App::new(None);

        let effect = app.dispatch_settings_action(SettingsDispatchAction::SetTextSpacingMode(
            ui::typography::TextSpacingMode::Verbatim,
        ));

        assert_eq!(
            app.editor_runtime.settings().text_spacing_mode,
            ui::typography::TextSpacingMode::Verbatim
        );
        assert!(effect.persist_settings);
        assert!(effect.reshape);
    }

    #[test]
    fn markdown_first_line_indent_returns_persist_and_reshape() {
        let mut app = App::new(None);
        let effect =
            app.dispatch_settings_action(SettingsDispatchAction::SetMarkdownFirstLineIndent(true));

        assert!(app.editor_runtime.settings().markdown_first_line_indent);
        assert!(effect.persist_settings);
        assert!(effect.reshape);
    }
}
