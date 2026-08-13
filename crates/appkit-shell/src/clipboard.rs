//! Shell-local system clipboard implementation.

pub struct SystemClipboard;

impl ui::core::Clipboard for SystemClipboard {
    fn read_text(&mut self) -> Option<String> {
        try_read_text()
    }

    fn write_text(&mut self, text: &str) -> bool {
        try_write_text(text)
    }
}

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
