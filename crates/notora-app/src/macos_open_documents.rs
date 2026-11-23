use std::ffi::CStr;

use crate::product::OpenDocumentSender;

const DELEGATE_SUBCLASS_NAME: &CStr = c"NotoraOpenDocumentApplicationDelegate";

/// 为当前 `NSApplication` 的 winit 委托注入 `application:openURLs:` 处理入口。
pub fn install_macos_open_document_handler(
    open_document_sender: OpenDocumentSender,
) -> Result<(), String> {
    appkit_shell::macos::install_open_document_handler(DELEGATE_SUBCLASS_NAME, move |paths| {
        open_document_sender.send(paths)
    })
}
