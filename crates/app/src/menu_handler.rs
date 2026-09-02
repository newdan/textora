//! Menu action dispatching — pure functions returning commands.
//!
//! Strategy 3 (Command pattern): handlers no longer take `&mut App`.
//! Instead they parse intent and return `AppCommand` values.
//! The caller (`App`) executes the returned commands in its own context.

use crate::input::EditCommand;
use crate::native_menu::MenuAction;

/// Commands that menu actions can produce.
/// Executed by `App::execute_commands`.
#[derive(Debug, Clone)]
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
    SetThemeModeSystem,
    SetThemeModeDark,
    SetThemeModeLight,
    ToggleLineNumbers,
    ToggleWordWrap,
    SetViewModeSidebar,
    SetViewModeTabs,
}

/// Map a native `MenuAction` to zero or more `AppCommand`s.
pub(crate) fn dispatch_menu_action(action: MenuAction) -> Vec<AppCommand> {
    match action {
        MenuAction::About => vec![AppCommand::Noop], // TODO: show about dialog
        MenuAction::Preferences => vec![AppCommand::OpenSettings],
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
    use super::{AppCommand, dispatch_menu_action};
    use crate::input::EditCommand;
    use crate::native_menu::MenuAction;

    #[test]
    fn paste_plain_text_menu_maps_to_edit_command() {
        let commands = dispatch_menu_action(MenuAction::PastePlainText);
        assert_eq!(commands.len(), 1);
        assert!(matches!(&commands[0], AppCommand::Edit(EditCommand::PastePlainText)));
    }
}
