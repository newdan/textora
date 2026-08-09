use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::persistence_worker::{PersistenceWorker, PersistenceWorkerDisconnected};
use crate::session::ProductSession;
use crate::settings::ProductSettings;

const SESSION_PERSIST_DEBOUNCE_DELAY: Duration = Duration::from_millis(300);
const CATALOG_BACKUP_DEBOUNCE_DELAY: Duration = Duration::from_millis(300);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum SettingsPersistenceState {
    #[default]
    Saved,
    SaveFailed {
        message: String,
    },
}

impl SettingsPersistenceState {
    pub(super) fn to_view(&self) -> crate::settings_overlay::NotoraSettingsPersistenceView {
        match self {
            Self::Saved => crate::settings_overlay::NotoraSettingsPersistenceView::Saved,
            Self::SaveFailed { message } => {
                crate::settings_overlay::NotoraSettingsPersistenceView::SaveFailed {
                    message: message.clone(),
                }
            }
        }
    }
}

/// settings、session 与 catalog backup 调度状态的唯一所有者。
pub(super) struct PersistenceRuntime {
    pub(super) product_settings: ProductSettings,
    pub(super) pending_session: Option<ProductSession>,
    pub(super) settings_state: SettingsPersistenceState,
    last_enqueued_settings: Option<ProductSettings>,
    pub(super) session_persist_at: Option<Instant>,
    pub(super) catalog_backup_at: Option<Instant>,
    pub(super) worker: PersistenceWorker,
}

impl PersistenceRuntime {
    pub(super) fn new(
        product_settings: ProductSettings,
        pending_session: ProductSession,
        worker: PersistenceWorker,
    ) -> Self {
        Self {
            product_settings,
            pending_session: Some(pending_session),
            settings_state: SettingsPersistenceState::Saved,
            last_enqueued_settings: None,
            session_persist_at: None,
            catalog_backup_at: None,
            worker,
        }
    }

    pub(super) fn schedule_session_persistence(&mut self, now: Instant) {
        self.session_persist_at = Some(now + SESSION_PERSIST_DEBOUNCE_DELAY);
    }

    pub(super) fn take_due_session_persistence(&mut self, now: Instant) -> bool {
        if self.session_persist_at.is_none_or(|deadline| deadline > now) {
            return false;
        }
        self.session_persist_at = None;
        true
    }

    pub(super) fn schedule_catalog_backup(&mut self, now: Instant) {
        self.catalog_backup_at = Some(now + CATALOG_BACKUP_DEBOUNCE_DELAY);
    }

    pub(super) fn take_due_catalog_backup(&mut self, now: Instant) -> bool {
        if self.catalog_backup_at.is_none_or(|deadline| deadline > now) {
            return false;
        }
        self.catalog_backup_at = None;
        true
    }

    pub(super) fn take_pending_catalog_backup(&mut self) -> bool {
        self.catalog_backup_at.take().is_some()
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        [self.session_persist_at, self.catalog_backup_at].into_iter().flatten().min()
    }

    pub(super) fn apply_settings_update(
        &mut self,
        update: crate::settings_overlay::ProductSettingsUpdate,
    ) {
        update.apply_to(&mut self.product_settings);
    }

    pub(super) fn save_settings(
        &mut self,
        path: PathBuf,
    ) -> Result<(), PersistenceWorkerDisconnected> {
        if self.last_enqueued_settings.as_ref() == Some(&self.product_settings) {
            return Ok(());
        }
        self.worker.save_settings(path, self.product_settings.clone())?;
        self.last_enqueued_settings = Some(self.product_settings.clone());
        Ok(())
    }

    pub(super) fn save_session(
        &self,
        path: PathBuf,
        session: ProductSession,
    ) -> Result<(), PersistenceWorkerDisconnected> {
        self.worker.save_session(path, session)
    }

    pub(super) fn record_settings_result(&mut self, result: Result<(), String>) {
        if result.is_err() {
            self.last_enqueued_settings = None;
        }
        self.settings_state = match result {
            Ok(()) => SettingsPersistenceState::Saved,
            Err(message) => SettingsPersistenceState::SaveFailed { message },
        };
    }

    pub(super) fn shutdown(&mut self) {
        self.worker.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use appkit_shell::ProductHost;

    use super::PersistenceRuntime;

    #[test]
    fn session_and_catalog_deadlines_are_owned_and_consumed_independently() {
        let mut product = crate::product::NotoraProduct::new();
        let worker = crate::persistence_worker::PersistenceWorker::start(product.event_sender())
            .expect("persistence worker should start");
        let mut runtime = PersistenceRuntime::new(
            crate::settings::ProductSettings::default(),
            crate::session::ProductSession::default(),
            worker,
        );
        let now = Instant::now();

        runtime.schedule_session_persistence(now);
        runtime.schedule_catalog_backup(now);
        assert!(!runtime.take_due_session_persistence(now + Duration::from_millis(299)));
        assert!(runtime.take_due_session_persistence(now + Duration::from_millis(300)));
        assert!(runtime.take_due_catalog_backup(now + Duration::from_millis(300)));
        assert_eq!(runtime.next_deadline(), None);

        runtime.shutdown();
        ProductHost::shutdown(&mut product);
    }

    #[test]
    fn identical_settings_snapshot_is_enqueued_only_once() {
        let directory = tempfile::tempdir().expect("settings directory should exist");
        let settings_path = directory.path().join("settings.toml");
        let mut product = crate::product::NotoraProduct::new();
        let worker = crate::persistence_worker::PersistenceWorker::start(product.event_sender())
            .expect("persistence worker should start");
        let mut runtime = PersistenceRuntime::new(
            crate::settings::ProductSettings::default(),
            crate::session::ProductSession::default(),
            worker,
        );

        runtime.save_settings(settings_path.clone()).expect("first snapshot should enqueue");
        runtime
            .save_settings(settings_path)
            .expect("identical snapshot should not enqueue a second command");
        runtime.shutdown();
        let _ = product.drain_product_events();

        assert_eq!(product.take_workspace_events().len(), 1);
        ProductHost::shutdown(&mut product);
    }
}
