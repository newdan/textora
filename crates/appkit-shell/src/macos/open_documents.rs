use std::ffi::CStr;
use std::path::PathBuf;
use std::sync::OnceLock;

use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use objc2::sel;
use objc2_app_kit::NSApplication;
use objc2_foundation::{MainThreadMarker, NSArray, NSURL};

use crate::ProductEventSendError;

type OpenDocumentDispatch = dyn Fn(Vec<PathBuf>) -> Result<(), ProductEventSendError> + Send + Sync;
static OPEN_DOCUMENT_DISPATCH: OnceLock<Box<OpenDocumentDispatch>> = OnceLock::new();

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
        let Some(dispatch) = OPEN_DOCUMENT_DISPATCH.get() else {
            eprintln!("[macos] open-document bridge is not installed");
            return;
        };
        if dispatch(paths).is_err() {
            eprintln!("[macos] open-document product inbox is unavailable");
        }
    }));
    if result.is_err() {
        eprintln!("[macos] panic while handling application:openURLs:");
    }
}

fn delegate_subclass(parent: &AnyClass, subclass_name: &CStr) -> Result<&'static AnyClass, String> {
    if let Some(existing) = AnyClass::get(subclass_name) {
        if existing.superclass() != Some(parent) {
            return Err("open-document delegate has an unexpected parent".to_owned());
        }
        return Ok(existing);
    }
    let mut builder = ClassBuilder::new(subclass_name, parent)
        .ok_or_else(|| "could not allocate open-document delegate".to_owned())?;
    // SAFETY: The callback matches the AppKit application:openURLs: selector signature.
    unsafe {
        builder.add_method::<AnyObject, _>(
            sel!(application:openURLs:),
            application_open_urls as unsafe extern "C-unwind" fn(_, _, _, _),
        );
    }
    Ok(builder.register())
}

/// Installs the process-wide Finder open-document handler on winit's AppKit delegate.
/// The subclass name must be unique in the Objective-C runtime.
pub fn install_open_document_handler(
    subclass_name: &CStr,
    dispatch: impl Fn(Vec<PathBuf>) -> Result<(), ProductEventSendError> + Send + Sync + 'static,
) -> Result<(), String> {
    let main_thread = MainThreadMarker::new()
        .ok_or_else(|| "open-document bridge requires the main thread".to_owned())?;
    if OPEN_DOCUMENT_DISPATCH.get().is_some() {
        return Err("open-document bridge is already installed".to_owned());
    }
    let application = NSApplication::sharedApplication(main_thread);
    let delegate =
        application.delegate().ok_or_else(|| "winit application delegate is missing".to_owned())?;
    let delegate_object = AsRef::<AnyObject>::as_ref(&*delegate);
    let parent = delegate_object.class();
    let subclass = delegate_subclass(parent, subclass_name)?;
    if subclass.instance_size() != parent.instance_size() {
        return Err("open-document delegate changes instance size".to_owned());
    }
    OPEN_DOCUMENT_DISPATCH
        .set(Box::new(dispatch))
        .map_err(|_| "open-document bridge is already installed".to_owned())?;
    // SAFETY: The subclass derives from the current class and preserves its instance layout.
    let previous = unsafe { AnyObject::set_class(delegate_object, subclass) };
    if previous != parent {
        return Err("application delegate changed during installation".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::NSString;

    #[test]
    fn file_urls_become_paths_and_non_file_urls_are_ignored() {
        let first_file = NSURL::fileURLWithPath(&NSString::from_str("/tmp/first document.md"));
        let second_file = NSURL::fileURLWithPath(&NSString::from_str("/tmp/second.md"));
        let web_url = NSURL::URLWithString(&NSString::from_str("https://example.com/doc.md"))
            .expect("the test URL is valid");
        let urls = NSArray::from_retained_slice(&[first_file, web_url, second_file]);

        assert_eq!(
            paths_from_urls(&urls),
            vec![PathBuf::from("/tmp/first document.md"), PathBuf::from("/tmp/second.md")]
        );
    }

    #[test]
    fn installer_configures_dispatch_after_validation_and_before_delegate_swap() {
        let source = include_str!("open_documents.rs");
        let (_, installer) = source
            .split_once("pub fn install_open_document_handler(")
            .expect("installer function must exist");
        let (installer, _) = installer
            .split_once("\n}\n\n#[cfg(test)]")
            .expect("installer function must end before its test module");

        let instance_size_validation = installer
            .find("if subclass.instance_size() != parent.instance_size()")
            .expect("installer must validate the delegate instance size");
        let dispatch_configuration = installer
            .find(".set(Box::new(dispatch))")
            .expect("installer must configure the open-document dispatch");
        let delegate_swap = installer
            .find("AnyObject::set_class(")
            .expect("installer must replace the application delegate class");

        assert!(instance_size_validation < dispatch_configuration);
        assert!(dispatch_configuration < delegate_swap);
    }
}
