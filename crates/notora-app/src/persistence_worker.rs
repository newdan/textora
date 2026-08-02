//! 设置与会话的串行后台持久化服务。

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use crate::product::{NotoraProductEvent, NotoraProductEventSender};

enum PersistenceCommand {
    SaveSettings { path: PathBuf, settings: crate::settings::ProductSettings },
    SaveSession { path: PathBuf, session: crate::session::ProductSession },
    Shutdown,
}

pub(crate) struct PersistenceWorker {
    command_sender: Option<mpsc::Sender<PersistenceCommand>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl PersistenceWorker {
    pub(crate) fn start(event_sender: NotoraProductEventSender) -> std::io::Result<Self> {
        let (command_sender, command_receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("notora-persistence".to_owned())
            .spawn(move || run_persistence_worker(command_receiver, event_sender))?;
        Ok(Self { command_sender: Some(command_sender), worker: Some(worker) })
    }

    pub(crate) fn save_settings(
        &self,
        path: PathBuf,
        settings: crate::settings::ProductSettings,
    ) -> Result<(), PersistenceWorkerDisconnected> {
        self.send(PersistenceCommand::SaveSettings { path, settings })
    }

    pub(crate) fn save_session(
        &self,
        path: PathBuf,
        session: crate::session::ProductSession,
    ) -> Result<(), PersistenceWorkerDisconnected> {
        self.send(PersistenceCommand::SaveSession { path, session })
    }

    pub(crate) fn shutdown(&mut self) {
        if let Some(sender) = self.command_sender.take() {
            let _ = sender.send(PersistenceCommand::Shutdown);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    fn send(&self, command: PersistenceCommand) -> Result<(), PersistenceWorkerDisconnected> {
        self.command_sender
            .as_ref()
            .ok_or(PersistenceWorkerDisconnected)?
            .send(command)
            .map_err(|_| PersistenceWorkerDisconnected)
    }
}

impl Drop for PersistenceWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PersistenceWorkerDisconnected;

impl std::fmt::Display for PersistenceWorkerDisconnected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the persistence worker is unavailable")
    }
}

fn run_persistence_worker(
    command_receiver: mpsc::Receiver<PersistenceCommand>,
    event_sender: NotoraProductEventSender,
) {
    while let Ok(command) = command_receiver.recv() {
        let result = match command {
            PersistenceCommand::SaveSettings { path, settings } => {
                crate::settings::save_product_settings(&path, &settings)
                    .map_err(|error| format!("could not persist product settings: {error}"))
            }
            PersistenceCommand::SaveSession { path, session } => {
                crate::session::save_product_session(&path, &session)
                    .map_err(|error| format!("could not persist product session: {error}"))
            }
            PersistenceCommand::Shutdown => return,
        };
        if let Err(message) = result {
            let _ = event_sender.send(NotoraProductEvent::PersistenceFailed { message });
        }
    }
}

#[cfg(test)]
mod tests {
    use appkit_shell::ProductHost;

    use super::PersistenceWorker;

    #[test]
    fn ordered_shutdown_flushes_the_latest_settings_snapshot() {
        let directory = tempfile::tempdir().expect("persistence directory should be created");
        let settings_path = directory.path().join("settings.toml");
        let mut product = crate::product::NotoraProduct::new();
        let mut worker =
            PersistenceWorker::start(product.event_sender()).expect("worker should start");
        let first = crate::settings::ProductSettings::default();
        let mut latest = first.clone();
        latest.editor.word_wrap = !first.editor.word_wrap;

        worker
            .save_settings(settings_path.clone(), first)
            .expect("first settings snapshot should enqueue");
        worker
            .save_settings(settings_path.clone(), latest.clone())
            .expect("latest settings snapshot should enqueue");
        worker.shutdown();

        assert_eq!(crate::settings::load_product_settings(&settings_path).settings, latest);
        assert_eq!(product.drain_product_events(), appkit_shell::ShellEffect::NONE);
    }
}
