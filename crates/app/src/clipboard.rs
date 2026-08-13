use appkit_shell::SystemClipboard;
use ui::core::Clipboard;

pub fn copy_to_clipboard(text: &str) -> bool {
    SystemClipboard.write_text(text)
}

pub fn paste_from_clipboard() -> Option<String> {
    SystemClipboard.read_text()
}

#[cfg(test)]
mod tests {
    #[test]
    fn platform_clipboard_implementation_is_owned_only_by_the_shared_shell() {
        let app_manifest = include_str!("../Cargo.toml");
        let app_sources = [
            include_str!("commands.rs"),
            include_str!("dispatch/editor.rs"),
            include_str!("document_view/selection.rs"),
            include_str!("workspace_product.rs"),
        ];
        let shell_clipboard_source = include_str!("../../appkit-shell/src/clipboard.rs");

        assert!(!app_manifest.lines().any(|line| line.trim_start().starts_with("arboard")));
        assert!(app_sources.iter().all(|source| !source.contains("arboard::")));
        assert!(shell_clipboard_source.contains("arboard::Clipboard"));
    }
}
