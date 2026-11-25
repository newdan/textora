use std::path::PathBuf;

use appkit_shell::{
    ProductEventInbox, ProductEventSender as SharedProductEventSender, ProductHost,
    ProductWakeHandle, ShellEffect, product_event_channel,
};

use crate::Edition;
use crate::native_menu::NativeMenu;
#[cfg(feature = "sync")]
use crate::sync_controller::SyncController;

enum ProductEvent {
    RecentFilesLoaded(Vec<PathBuf>),
    #[cfg(feature = "sync")]
    SyncResultsReady,
    OpenDocumentsRequested(Vec<PathBuf>),
}

#[derive(Clone)]
pub(crate) struct ProductEventSender {
    sender: SharedProductEventSender<ProductEvent>,
}

impl ProductEventSender {
    pub(crate) fn send_recent_files_loaded(
        &self,
        recent_paths: Vec<PathBuf>,
    ) -> Result<(), ProductEventSendError> {
        self.sender.send(ProductEvent::RecentFilesLoaded(recent_paths))
    }

    #[cfg(feature = "sync")]
    pub(crate) fn send_sync_results_ready(&self) -> Result<(), ProductEventSendError> {
        self.sender.send(ProductEvent::SyncResultsReady)
    }
}

#[derive(Clone)]
pub struct OpenDocumentSender {
    sender: SharedProductEventSender<ProductEvent>,
}

impl OpenDocumentSender {
    pub(crate) fn send(&self, paths: Vec<PathBuf>) -> Result<(), ProductEventSendError> {
        self.sender.send(ProductEvent::OpenDocumentsRequested(paths))
    }
}

pub(crate) type ProductEventSendError = appkit_shell::ProductEventSendError;

pub(crate) struct TextoraProduct {
    edition: Edition,
    #[cfg(feature = "sync")]
    sync_controller: Option<SyncController>,
    native_menu: Option<NativeMenu>,
    event_sender: ProductEventSender,
    event_inbox: ProductEventInbox<ProductEvent>,
    open_document_sender: OpenDocumentSender,
    pending_open_document_paths: Vec<PathBuf>,
}

impl TextoraProduct {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::for_edition(Edition::Standard)
    }

    pub(crate) fn for_edition(edition: Edition) -> Self {
        let (event_sender, event_inbox) = product_event_channel();

        Self {
            edition,
            #[cfg(feature = "sync")]
            sync_controller: None,
            native_menu: None,
            event_sender: ProductEventSender { sender: event_sender.clone() },
            event_inbox,
            open_document_sender: OpenDocumentSender { sender: event_sender },
            pending_open_document_paths: Vec::new(),
        }
    }

    pub(crate) fn edition(&self) -> Edition {
        self.edition
    }

    pub(crate) fn event_sender(&self) -> ProductEventSender {
        self.event_sender.clone()
    }

    pub(crate) fn open_document_sender(&self) -> OpenDocumentSender {
        self.open_document_sender.clone()
    }

    pub(crate) fn drain_open_documents(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.pending_open_document_paths)
    }

    #[cfg(feature = "sync")]
    pub(crate) fn sync_controller(&self) -> Option<&SyncController> {
        self.sync_controller.as_ref()
    }

    #[cfg(feature = "sync")]
    pub(crate) fn sync_controller_mut(&mut self) -> Option<&mut SyncController> {
        self.sync_controller.as_mut()
    }

    #[cfg(feature = "sync")]
    pub(crate) fn set_sync_controller(&mut self, controller: SyncController) {
        self.sync_controller = Some(controller);
    }

    #[cfg(feature = "sync")]
    pub(crate) fn take_sync_controller(&mut self) -> Option<SyncController> {
        self.sync_controller.take()
    }

    pub(crate) fn native_menu(&self) -> Option<&NativeMenu> {
        self.native_menu.as_ref()
    }

    pub(crate) fn set_native_menu(&mut self, native_menu: NativeMenu) {
        self.native_menu = Some(native_menu);
    }
}

impl ProductHost for TextoraProduct {
    fn start_background_services(&mut self, wake: ProductWakeHandle) {
        let _ = self.event_inbox.register_wake_handle(wake);
        #[cfg(feature = "sync")]
        if self.edition == Edition::Standard && self.sync_controller.is_none() {
            let event_sender = self.event_sender();
            self.set_sync_controller(SyncController::new_default(move || {
                let _ = event_sender.send_sync_results_ready();
            }));
        }
    }

    fn drain_product_events(&mut self) -> ShellEffect {
        let mut effect = ShellEffect::NONE;

        for event in self.event_inbox.drain() {
            let event_effect = match event {
                ProductEvent::RecentFilesLoaded(recent_paths) => {
                    self.set_native_menu(NativeMenu::build(&recent_paths, self.edition));
                    ShellEffect::NONE
                }
                #[cfg(feature = "sync")]
                ProductEvent::SyncResultsReady => {
                    if let Some(controller) = self.sync_controller_mut() {
                        controller.drain_background();
                    }
                    ShellEffect::REDRAW
                }
                ProductEvent::OpenDocumentsRequested(paths) => {
                    self.pending_open_document_paths.extend(paths);
                    ShellEffect::NONE
                }
            };
            effect = effect.merge(event_effect);
        }

        effect
    }

    fn shutdown(&mut self) {
        #[cfg(feature = "sync")]
        if let Some(controller) = self.take_sync_controller() {
            controller.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use appkit_shell::{ProductHost, ShellEffect};

    use super::TextoraProduct;

    #[test]
    fn open_document_inbox_preserves_path_order() {
        let mut product = TextoraProduct::new();
        product
            .open_document_sender()
            .send(vec![PathBuf::from("/tmp/a.md"), PathBuf::from("/tmp/b.txt")])
            .expect("product receiver is alive");
        let _ = ProductHost::drain_product_events(&mut product);

        assert_eq!(
            product.drain_open_documents(),
            vec![PathBuf::from("/tmp/a.md"), PathBuf::from("/tmp/b.txt")]
        );
    }

    #[test]
    #[cfg(feature = "sync")]
    fn sync_completion_drains_to_redraw() {
        let mut product = TextoraProduct::new();
        product.event_sender().send_sync_results_ready().expect("product receiver is alive");

        assert_eq!(product.drain_product_events(), ShellEffect::REDRAW);
    }
}
