//! Menu action dispatching — pure functions returning commands.
//!
//! Strategy 3 (Command pattern): handlers no longer take `&mut App`.
//! Instead they parse intent and return `AppCommand` values.
//! The caller (`App`) executes the returned commands in its own context.

use crate::native_menu::MenuAction;
use appkit_core::edit_command::EditCommand;

/// Commands routed by the Textora product.
/// Executed by `App::execute_commands`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum AppCommand {
    /// No operation.
    Noop,
    /// Exit the application.
    Quit,
    /// Create a new empty tab.
    NewEmptyTab,
    /// Open the file dialog.
    OpenFileDialog,
    /// Save the active document.
    SaveActiveTab,
    /// Save the active document (as-if, with dialog).
    SaveActiveTabAs,
    /// Close the tab at the given index.
    CloseTab(usize),
    /// Close all tabs except the given index.
    CloseOthers(usize),
    /// Close all tabs to the right of the given index.
    CloseRight(usize),
    /// Close all non-pinned tabs.
    CloseAll,
    /// Toggle pin for the active tab.
    TogglePin,
    /// Toggle search panel.
    ToggleFind,
    /// Execute an edit command on the active document.
    Edit(EditCommand),
    /// A keyboard or editor-chrome command with editor focus and access rules.
    EditorShortcut(EditorShortcut),
    /// Increase font size.
    ZoomIn,
    /// Decrease font size.
    ZoomOut,
    /// Reset font size to default.
    ZoomReset,
    /// Open a file from the recent files list.
    OpenRecentFile(usize),
    /// Clear all recent file history.
    ClearRecentFiles,
    /// Toggle status bar visibility.
    ToggleStatusBar,
    /// Toggle tab bar visibility.
    ToggleTabBar,
    /// Open the settings file.
    OpenSettings,
    /// Open the Markdown table size picker.
    OpenTablePicker,
    SetThemeModeSystem,
    SetThemeModeDark,
    SetThemeModeLight,
    ToggleLineNumbers,
    ToggleWordWrap,
    SetViewModeSidebar,
    SetViewModeTabs,
}

/// Product commands sharing the editor keyboard dispatch rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorShortcut {
    OpenFile,
    NewTab,
    CloseTab,
    ToggleSidebarPin,
    ToggleView,
    ToggleToc,
    NextTab,
    PrevTab,
    SwitchTab(usize),
    NavigateBack,
    NavigateForward,
    NextChapter,
    PrevChapter,
    Escape,
    Find,
    FindReplace,
    FindNext,
    FindPrev,
}

impl EditorShortcut {
    pub(crate) fn allowed_in_read_only(self) -> bool {
        matches!(
            self,
            Self::ToggleView
                | Self::Escape
                | Self::CloseTab
                | Self::NextTab
                | Self::PrevTab
                | Self::NavigateBack
                | Self::NavigateForward
                | Self::ToggleSidebarPin
                | Self::ToggleToc
                | Self::Find
                | Self::FindNext
                | Self::FindPrev
                | Self::NextChapter
                | Self::PrevChapter
        )
    }

    pub(crate) fn breaks_edit_merge(self) -> bool {
        matches!(self, Self::NavigateBack | Self::NavigateForward)
    }
}

/// Map a native `MenuAction` to zero or more `AppCommand`s.
pub(crate) fn dispatch_menu_action(action: MenuAction) -> Vec<AppCommand> {
    match action {
        MenuAction::About => vec![AppCommand::Noop], // TODO: show about dialog
        MenuAction::Preferences => vec![AppCommand::OpenSettings],
        MenuAction::InsertTable => vec![AppCommand::OpenTablePicker],
        MenuAction::ToggleTabBar => vec![AppCommand::ToggleTabBar],
        MenuAction::ToggleStatusBar => vec![AppCommand::ToggleStatusBar],
        MenuAction::Find => vec![AppCommand::ToggleFind],
        MenuAction::Quit => vec![AppCommand::Quit],
        MenuAction::NewFile => vec![AppCommand::NewEmptyTab],
        MenuAction::OpenFile => vec![AppCommand::OpenFileDialog],
        MenuAction::Save => vec![AppCommand::SaveActiveTab],
        MenuAction::SaveAs => vec![AppCommand::SaveActiveTabAs],
        MenuAction::CloseTab => vec![AppCommand::CloseTab(0)], // index resolved by caller
        MenuAction::Undo => vec![AppCommand::Edit(EditCommand::Undo)],
        MenuAction::Redo => vec![AppCommand::Edit(EditCommand::Redo)],
        MenuAction::Cut => vec![AppCommand::Edit(EditCommand::Cut)],
        MenuAction::Copy => vec![AppCommand::Edit(EditCommand::Copy)],
        MenuAction::Paste => vec![AppCommand::Edit(EditCommand::Paste)],
        MenuAction::PastePlainText => vec![AppCommand::Edit(EditCommand::PastePlainText)],
        MenuAction::SelectAll => vec![AppCommand::Edit(EditCommand::SelectAll)],
        MenuAction::ZoomIn => vec![AppCommand::ZoomIn],
        MenuAction::ZoomOut => vec![AppCommand::ZoomOut],
        MenuAction::ZoomReset => vec![AppCommand::ZoomReset],
        MenuAction::OpenRecentFile(idx) => vec![AppCommand::OpenRecentFile(idx)],
        MenuAction::ClearRecentFiles => vec![AppCommand::ClearRecentFiles],
        MenuAction::SetThemeModeSystem => vec![AppCommand::SetThemeModeSystem],
        MenuAction::SetThemeModeDark => vec![AppCommand::SetThemeModeDark],
        MenuAction::SetThemeModeLight => vec![AppCommand::SetThemeModeLight],
        MenuAction::ToggleLineNumbers => vec![AppCommand::ToggleLineNumbers],
        MenuAction::ToggleWordWrap => vec![AppCommand::ToggleWordWrap],
        MenuAction::SetViewModeSidebar => vec![AppCommand::SetViewModeSidebar],
        MenuAction::SetViewModeTabs => vec![AppCommand::SetViewModeTabs],
    }
}

#[cfg(test)]
mod tests {
    use super::{AppCommand, EditorShortcut, dispatch_menu_action};
    use crate::native_menu::MenuAction;
    use appkit_core::edit_command::EditCommand;

    #[test]
    fn native_find_opens_or_focuses_while_keyboard_find_toggles() {
        use winit::keyboard::{Key, ModifiersState};

        assert_eq!(dispatch_menu_action(MenuAction::Find), [AppCommand::ToggleFind]);
        assert_eq!(
            crate::input_mapper::key_to_app_command(
                &Key::Character("f".into()),
                ModifiersState::SUPER
            ),
            Some(AppCommand::EditorShortcut(EditorShortcut::Find)),
        );
    }

    #[test]
    fn product_shortcuts_keep_read_only_access_and_history_merge_rules() {
        use EditorShortcut::*;
        for command in [OpenFile, NewTab, SwitchTab(1), FindReplace] {
            assert!(!command.allowed_in_read_only(), "{command:?}");
        }
        for command in [
            CloseTab,
            NextTab,
            PrevTab,
            ToggleView,
            ToggleToc,
            ToggleSidebarPin,
            NavigateBack,
            NavigateForward,
            NextChapter,
            PrevChapter,
            Escape,
            Find,
            FindNext,
            FindPrev,
        ] {
            assert!(command.allowed_in_read_only(), "{command:?}");
            assert_eq!(
                command.breaks_edit_merge(),
                matches!(command, NavigateBack | NavigateForward)
            );
        }
    }

    #[test]
    fn paste_plain_text_menu_maps_to_edit_command() {
        let commands = dispatch_menu_action(MenuAction::PastePlainText);
        assert_eq!(commands.len(), 1);
        assert!(matches!(&commands[0], AppCommand::Edit(EditCommand::PastePlainText)));
    }

    #[test]
    fn insert_table_menu_opens_the_table_picker() {
        let commands = dispatch_menu_action(MenuAction::InsertTable);
        assert!(matches!(commands.as_slice(), [AppCommand::OpenTablePicker]));
    }
}
