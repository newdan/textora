use arboard::Clipboard;
use std::sync::Mutex;
use std::sync::OnceLock;

static CLIPBOARD: OnceLock<Mutex<Option<Clipboard>>> = OnceLock::new();

fn get_clipboard() -> std::sync::MutexGuard<'static, Option<Clipboard>> {
    let mut lock = CLIPBOARD.get_or_init(|| Mutex::new(Clipboard::new().ok())).lock().unwrap();

    // If clipboard failed to initialize previously, try again
    if lock.is_none() {
        *lock = Clipboard::new().ok();
    }

    lock
}

pub fn copy_to_clipboard(text: &str) -> bool {
    let mut cb = get_clipboard();
    if let Some(clipboard) = cb.as_mut() { clipboard.set_text(text).is_ok() } else { false }
}

pub fn paste_from_clipboard() -> Option<String> {
    let mut cb = get_clipboard();
    if let Some(clipboard) = cb.as_mut() { clipboard.get_text().ok() } else { None }
}
