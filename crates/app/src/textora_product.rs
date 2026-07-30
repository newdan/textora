use std::fmt;
use std::path::PathBuf;
use std::sync::mpsc;

use appkit_shell::{ProductHost, ProductWakeHandle, ShellEffect};

use crate::native_menu::NativeMenu;
use crate::sync_controller::SyncController;

enum ProductEvent {
    RecentFilesLoaded(Vec<PathBuf>),
    SyncResultsReady,
}

#[derive(Clone)]
pub(crate) struct ProductEventSender {
    sender: mpsc::Sender<ProductEvent>,
}

impl ProductEventSender {
    pub(crate) fn send_recent_files_loaded(
        &self,
        recent_paths: Vec<PathBuf>,
    ) -> Result<(), ProductEventSendError> {
        self.sender
            .send(ProductEvent::RecentFilesLoaded(recent_paths))
            .map_err(|_| ProductEventSendError)
    }

    pub(crate) fn send_sync_results_ready(&self) -> Result<(), ProductEventSendError> {
        self.sender.send(ProductEvent::SyncResultsReady).map_err(|_| ProductEventSendError)
    }
}

#[derive(Clone)]
pub struct OpenDocumentSender {
    sender: mpsc::Sender<Vec<PathBuf>>,
}

impl OpenDocumentSender {
    pub(crate) fn send(&self, paths: Vec<PathBuf>) -> Result<(), ProductEventSendError> {
        self.sender.send(paths).map_err(|_| ProductEventSendError)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProductEventSendError;

impl fmt::Display for ProductEventSendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("product event receiver is unavailable")
    }
}

impl std::error::Error for ProductEventSendError {}

fn enqueue_sync_completion_and_wake(
    event_sender: &ProductEventSender,
    wake: impl FnOnce(),
) -> Result<(), ProductEventSendError> {
    event_sender.send_sync_results_ready()?;
    wake();
    Ok(())
}

pub(crate) struct TextoraProduct {
    sync_controller: Option<SyncController>,
    native_menu: Option<NativeMenu>,
    event_sender: ProductEventSender,
    event_receiver: mpsc::Receiver<ProductEvent>,
    open_document_sender: OpenDocumentSender,
    open_document_receiver: mpsc::Receiver<Vec<PathBuf>>,
}

impl TextoraProduct {
    pub(crate) fn new() -> Self {
        let (event_sender, event_receiver) = mpsc::channel();
        let (open_document_sender, open_document_receiver) = mpsc::channel();

        Self {
            sync_controller: None,
            native_menu: None,
            event_sender: ProductEventSender { sender: event_sender },
            event_receiver,
            open_document_sender: OpenDocumentSender { sender: open_document_sender },
            open_document_receiver,
        }
    }

    pub(crate) fn event_sender(&self) -> ProductEventSender {
        self.event_sender.clone()
    }

    pub(crate) fn open_document_sender(&self) -> OpenDocumentSender {
        self.open_document_sender.clone()
    }

    pub(crate) fn drain_open_documents(&mut self) -> Vec<PathBuf> {
        self.open_document_receiver.try_iter().flatten().collect()
    }

    pub(crate) fn sync_controller(&self) -> Option<&SyncController> {
        self.sync_controller.as_ref()
    }

    pub(crate) fn sync_controller_mut(&mut self) -> Option<&mut SyncController> {
        self.sync_controller.as_mut()
    }

    pub(crate) fn set_sync_controller(&mut self, controller: SyncController) {
        self.sync_controller = Some(controller);
    }

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
        if self.sync_controller.is_some() {
            return;
        }

        let event_sender = self.event_sender();
        self.set_sync_controller(SyncController::new_default(move || {
            let _ = enqueue_sync_completion_and_wake(&event_sender, || {
                let _ = wake.wake();
            });
        }));
    }

    fn drain_product_events(&mut self) -> ShellEffect {
        let mut effect = ShellEffect::NONE;

        while let Ok(event) = self.event_receiver.try_recv() {
            match event {
                ProductEvent::RecentFilesLoaded(recent_paths) => {
                    self.set_native_menu(NativeMenu::build(&recent_paths));
                }
                ProductEvent::SyncResultsReady => {
                    if let Some(controller) = self.sync_controller_mut() {
                        controller.drain_background();
                    }
                    effect = effect.merge(ShellEffect::REDRAW);
                }
            }
        }

        effect
    }

    fn shutdown(&mut self) {
        if let Some(controller) = self.take_sync_controller() {
            controller.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;

    use appkit_shell::{ProductHost, ShellEffect};

    use super::{TextoraProduct, enqueue_sync_completion_and_wake};

    #[test]
    fn open_document_inbox_preserves_path_order() {
        let mut product = TextoraProduct::new();
        product
            .open_document_sender()
            .send(vec![PathBuf::from("/tmp/a.md"), PathBuf::from("/tmp/b.txt")])
            .expect("product receiver is alive");

        assert_eq!(
            product.drain_open_documents(),
            vec![PathBuf::from("/tmp/a.md"), PathBuf::from("/tmp/b.txt")]
        );
    }

    #[test]
    fn sync_completion_drains_to_redraw() {
        let mut product = TextoraProduct::new();
        product.event_sender().send_sync_results_ready().expect("product receiver is alive");

        assert_eq!(product.drain_product_events(), ShellEffect::REDRAW);
    }

    #[test]
    fn sync_completion_is_enqueued_before_exactly_one_wake() {
        let mut product = TextoraProduct::new();
        let event_sender = product.event_sender();
        let wake_count = Cell::new(0);

        enqueue_sync_completion_and_wake(&event_sender, || {
            wake_count.set(wake_count.get() + 1);
            assert_eq!(product.drain_product_events(), ShellEffect::REDRAW);
        })
        .expect("product receiver is alive");

        assert_eq!(wake_count.get(), 1);
        assert_eq!(product.drain_product_events(), ShellEffect::NONE);
    }
}
