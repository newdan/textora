use std::ffi::CStr;
use std::path::PathBuf;
use std::sync::OnceLock;

use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use objc2::sel;
use objc2_app_kit::NSApplication;
use objc2_foundation::{MainThreadMarker, NSArray, NSURL};

use appkit_shell::ProductWakeHandle;

use crate::textora_product::OpenDocumentSender;

const DELEGATE_SUBCLASS_NAME: &CStr = c"TextoraOpenDocumentApplicationDelegate";
static OPEN_DOCUMENT_BRIDGE: OnceLock<OpenDocumentBridge> = OnceLock::new();

struct OpenDocumentBridge {
    open_document_sender: OpenDocumentSender,
    product_wake: ProductWakeHandle,
}

fn paths_from_urls(urls: &NSArray<NSURL>) -> Vec<PathBuf> {
    urls.iter()
        .filter(|url| url.isFileURL())
        .filter_map(|url| match url.path() {
            Some(path) => Some(path),
            None => {
                eprintln!("[macos] file URL does not contain a path");
                None
            }
        })
        .map(|path| PathBuf::from(path.to_string()))
        .collect()
}

fn dispatch_open_document_paths<Wake, WakeError>(
    open_document_sender: &OpenDocumentSender,
    paths: Vec<PathBuf>,
    send_wake: Wake,
) where
    Wake: FnOnce() -> Result<(), WakeError>,
{
    if open_document_sender.send(paths).is_err() {
        eprintln!("[macos] open-document product inbox is unavailable");
        return;
    }

    if send_wake().is_err() {
        eprintln!("[macos] open-document event loop is unavailable");
    }
}

unsafe extern "C-unwind" fn application_open_urls(
    _delegate: &AnyObject,
    _selector: Sel,
    _application: &NSApplication,
    urls: &NSArray<NSURL>,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let paths = paths_from_urls(urls);
        if paths.is_empty() {
            return;
        }
        let Some(bridge) = OPEN_DOCUMENT_BRIDGE.get() else {
            eprintln!("[macos] open-document bridge is not installed");
            return;
        };
        dispatch_open_document_paths(&bridge.open_document_sender, paths, || {
            bridge.product_wake.wake()
        });
    }));
    if result.is_err() {
        eprintln!("[macos] panic while handling application:openURLs:");
    }
}

fn delegate_subclass(parent: &AnyClass) -> Result<&'static AnyClass, String> {
    if let Some(existing) = AnyClass::get(DELEGATE_SUBCLASS_NAME) {
        if existing.superclass() != Some(parent) {
            return Err("open-document delegate has an unexpected parent".to_owned());
        }
        return Ok(existing);
    }
    let mut builder = ClassBuilder::new(DELEGATE_SUBCLASS_NAME, parent)
        .ok_or_else(|| "could not allocate open-document delegate".to_owned())?;
    unsafe {
        builder.add_method::<AnyObject, _>(
            sel!(application:openURLs:),
            application_open_urls as unsafe extern "C-unwind" fn(_, _, _, _),
        );
    }
    Ok(builder.register())
}

pub(crate) fn configure_macos_open_document_bridge(
    product_wake: ProductWakeHandle,
    open_document_sender: OpenDocumentSender,
) -> Result<(), String> {
    OPEN_DOCUMENT_BRIDGE
        .set(OpenDocumentBridge { open_document_sender, product_wake })
        .map_err(|_| "open-document bridge is already installed".to_owned())
}

pub fn install_macos_open_document_handler(
    product_wake: ProductWakeHandle,
    open_document_sender: OpenDocumentSender,
) -> Result<(), String> {
    let main_thread = MainThreadMarker::new()
        .ok_or_else(|| "open-document bridge requires the main thread".to_owned())?;
    let application = NSApplication::sharedApplication(main_thread);
    let delegate =
        application.delegate().ok_or_else(|| "winit application delegate is missing".to_owned())?;
    let delegate_object = AsRef::<AnyObject>::as_ref(&*delegate);
    let parent = delegate_object.class();
    let subclass = delegate_subclass(parent)?;
    if subclass.instance_size() != parent.instance_size() {
        return Err("open-document delegate changes instance size".to_owned());
    }
    configure_macos_open_document_bridge(product_wake, open_document_sender)?;
    let previous = unsafe { AnyObject::set_class(delegate_object, subclass) };
    if previous != parent {
        return Err("application delegate changed during installation".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{dispatch_open_document_paths, paths_from_urls};
    use crate::textora_product::TextoraProduct;
    use objc2_foundation::{NSArray, NSString, NSURL};
    use std::path::PathBuf;

    #[test]
    fn file_urls_become_paths_and_non_file_urls_are_ignored() {
        let file_url = NSURL::fileURLWithPath(&NSString::from_str("/tmp/textora document.md"));
        let web_url = NSURL::URLWithString(&NSString::from_str("https://example.com/doc.md"))
            .expect("the test URL is valid");
        let urls = NSArray::from_retained_slice(&[file_url, web_url]);

        assert_eq!(paths_from_urls(&urls), vec![PathBuf::from("/tmp/textora document.md")]);
    }

    #[test]
    fn dispatched_open_documents_enter_product_inbox_before_one_wake() {
        let mut product = TextoraProduct::new();
        let mut wake_count = 0;
        let paths = vec![PathBuf::from("/tmp/first.md"), PathBuf::from("/tmp/second.md")];

        dispatch_open_document_paths(&product.open_document_sender(), paths.clone(), || {
            wake_count += 1;
            Ok::<_, ()>(())
        });

        assert_eq!(wake_count, 1);
        assert_eq!(product.drain_open_documents(), paths);
    }

    #[test]
    fn open_document_bridge_keeps_only_typed_product_wake_handle() {
        let source = include_str!("macos_open_documents.rs");
        let bridge_start =
            source.find("struct OpenDocumentBridge").expect("open-document bridge must exist");
        let bridge_end = source[bridge_start..]
            .find("\n}\n\nfn paths_from_urls")
            .map(|offset| bridge_start + offset)
            .expect("open-document bridge must end before path conversion");
        let bridge = &source[bridge_start..bridge_end];
        let raw_event_loop_proxy = ["EventLoop", "Proxy<AppEvent>"].concat();

        assert!(bridge.contains("ProductWakeHandle"));
        assert!(
            !bridge.contains(&raw_event_loop_proxy),
            "open-document bridge must not retain the raw event-loop proxy"
        );
    }

    #[test]
    fn installer_configures_bridge_after_validation_and_before_delegate_swap() {
        let source = include_str!("macos_open_documents.rs");
        let installer_start = source
            .find("pub fn install_macos_open_document_handler(")
            .expect("installer function must exist");
        let installer_end = source[installer_start..]
            .find("\n}\n\n#[cfg(test)]")
            .map(|offset| installer_start + offset)
            .expect("installer function must end before its test module");
        let installer = &source[installer_start..installer_end];

        let instance_size_validation = installer
            .find("if subclass.instance_size() != parent.instance_size()")
            .expect("installer must validate the delegate instance size");
        let bridge_configuration = installer
            .find("configure_macos_open_document_bridge(")
            .expect("installer must configure the open-document bridge");
        let delegate_swap = installer
            .find("AnyObject::set_class(")
            .expect("installer must replace the application delegate class");

        assert!(
            instance_size_validation < bridge_configuration,
            "installer must validate the delegate instance size before configuring the bridge"
        );
        assert!(
            bridge_configuration < delegate_swap,
            "installer must configure the bridge before swapping the delegate class"
        );
    }
}
