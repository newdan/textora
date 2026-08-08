//! Shell-local clipboard access for UI callbacks.

pub(crate) fn try_write_text(text: &str) -> bool {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        clipboard.set_text(text).is_ok()
    } else {
        false
    }
}

pub(crate) fn try_read_text() -> Option<String> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    clipboard.get_text().ok()
}

pub(crate) fn write_text(text: String) {
    let _ = try_write_text(&text);
}

pub(crate) fn read_text() -> String {
    try_read_text().unwrap_or_default()
}
