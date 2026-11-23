use std::ffi::CStr;

use crate::textora_product::OpenDocumentSender;

const DELEGATE_SUBCLASS_NAME: &CStr = c"TextoraOpenDocumentApplicationDelegate";

pub fn install_macos_open_document_handler(
    open_document_sender: OpenDocumentSender,
) -> Result<(), String> {
    appkit_shell::macos::install_open_document_handler(DELEGATE_SUBCLASS_NAME, move |paths| {
        open_document_sender.send(paths)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn open_document_bridge_uses_the_typed_product_sender() {
        let (installer, _) = include_str!("macos_open_documents.rs")
            .split_once("\n#[cfg(test)]")
            .expect("the installer must precede its test module");
        let raw_event_loop_proxy = ["EventLoop", "Proxy<AppEvent>"].concat();

        assert!(installer.contains("open_document_sender: OpenDocumentSender"));
        assert!(installer.contains("open_document_sender.send(paths)"));
        assert!(!installer.contains("ProductWakeHandle"));
        assert!(!installer.contains(&raw_event_loop_proxy));
    }
}
