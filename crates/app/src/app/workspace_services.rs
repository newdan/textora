use std::collections::HashSet;

use appkit_core::file_history::{FileHistory, FileHistoryEntry};
use appkit_core::file_safety::FileSafetyNotice;
use appkit_core::tab_set::{store::WorkspaceStore, types::TabId};

use crate::library_file_monitor::LibraryFileMonitor;
use crate::product_paths::ProductPaths;

pub(crate) struct WorkspaceServices {
    store: WorkspaceStore,
    history: FileHistory,
    monitor: Option<LibraryFileMonitor>,
    file_safety_notices: Vec<FileSafetyNotice>,
    tabs_to_close_after_save: HashSet<TabId>,
    quit_after_save: bool,
}

impl WorkspaceServices {
    pub(crate) fn new(paths: &ProductPaths) -> Self {
        Self {
            store: WorkspaceStore::new(
                paths.workspace_file.clone(),
                paths.pinned_paths_file.clone(),
                paths.snapshots_dir.clone(),
            ),
            history: FileHistory::load(&paths.history_file).unwrap_or_default(),
            monitor: None,
            file_safety_notices: Vec::new(),
            tabs_to_close_after_save: HashSet::new(),
            quit_after_save: false,
        }
    }

    pub(crate) fn store(&self) -> &WorkspaceStore {
        &self.store
    }

    pub(crate) fn history(&self) -> &FileHistory {
        &self.history
    }

    pub(crate) fn record_history(&mut self, entry: FileHistoryEntry) {
        self.history.record(entry);
    }

    pub(crate) fn record_history_batch(&mut self, entries: Vec<FileHistoryEntry>) {
        self.history.record_batch(entries);
    }

    pub(crate) fn clear_history(&mut self) {
        self.history.entries.clear();
    }

    pub(crate) fn monitor(&self) -> Option<&LibraryFileMonitor> {
        self.monitor.as_ref()
    }

    pub(crate) fn install_monitor(&mut self, monitor: LibraryFileMonitor) {
        self.monitor = Some(monitor);
    }

    pub(crate) fn stop_monitor(&mut self) {
        if let Some(monitor) = self.monitor.take() {
            monitor.shutdown();
        }
    }

    pub(crate) fn record_file_safety_notice(&mut self, notice: FileSafetyNotice) {
        self.file_safety_notices.push(notice);
    }

    pub(crate) fn file_safety_notices(&self) -> &[FileSafetyNotice] {
        &self.file_safety_notices
    }

    pub(crate) fn await_tab_close_after_save(&mut self, tab_id: TabId) {
        self.tabs_to_close_after_save.insert(tab_id);
    }

    pub(crate) fn take_pending_tab_close(&mut self, tab_id: TabId) -> bool {
        self.tabs_to_close_after_save.remove(&tab_id)
    }

    pub(crate) fn has_pending_tab_closes(&self) -> bool {
        !self.tabs_to_close_after_save.is_empty()
    }

    pub(crate) fn set_quit_after_save(&mut self, pending: bool) {
        self.quit_after_save = pending;
    }

    pub(crate) fn quit_after_save(&self) -> bool {
        self.quit_after_save
    }
}
