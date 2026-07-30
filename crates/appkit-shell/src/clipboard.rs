//! Shell-local clipboard access for UI callbacks.

pub(crate) fn write_text(text: String) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text);
    }
}

pub(crate) fn read_text() -> String {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        clipboard.get_text().unwrap_or_default()
    } else {
        String::new()
    }
}
