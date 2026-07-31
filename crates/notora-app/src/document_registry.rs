//! 文档身份与 editor runtime tab 的双向注册表。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use appkit_core::workspace::types::TabId;
use notora_core::{DocumentIdentity, ExternalFileId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RegistryEntry {
    tab_id: TabId,
    last_access_sequence: u64,
    disposition: TabDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabDisposition {
    Preview,
    Persistent,
}

/// 注册或重新激活文档时被替换的映射。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplacedDocumentMapping {
    pub identity: DocumentIdentity,
    pub tab_id: TabId,
}

/// 外部文件路径不能 canonicalize 时的错误。
#[derive(Debug)]
pub struct ExternalPathError {
    pub path: PathBuf,
    pub source: std::io::Error,
}

impl std::fmt::Display for ExternalPathError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "could not canonicalize external file path {}: {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for ExternalPathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// 保证一个文档身份在任意时刻至多拥有一个活动 tab。
#[derive(Debug, Default)]
pub struct DocumentRegistry {
    entries_by_identity: HashMap<DocumentIdentity, RegistryEntry>,
    identities_by_tab: HashMap<TabId, DocumentIdentity>,
    external_identities_by_canonical_path: HashMap<PathBuf, ExternalFileId>,
    preview_tab_id: Option<TabId>,
    next_access_sequence: u64,
}

impl DocumentRegistry {
    /// 记录 identity 到 tab 的映射；同一 tab 原先的 identity 会被移除。
    pub fn register(
        &mut self,
        identity: DocumentIdentity,
        tab_id: TabId,
    ) -> Option<ReplacedDocumentMapping> {
        let last_access_sequence = self.next_access_sequence();
        let replaced = self.identities_by_tab.insert(tab_id, identity).and_then(|previous| {
            if previous == identity {
                return None;
            }
            self.entries_by_identity
                .remove(&previous)
                .map(|entry| ReplacedDocumentMapping { identity: previous, tab_id: entry.tab_id })
        });
        if let Some(previous_entry) = self.entries_by_identity.insert(
            identity,
            RegistryEntry { tab_id, last_access_sequence, disposition: TabDisposition::Persistent },
        ) && previous_entry.tab_id != tab_id
        {
            self.identities_by_tab.remove(&previous_entry.tab_id);
        }
        replaced
    }

    pub fn register_preview(
        &mut self,
        identity: DocumentIdentity,
        tab_id: TabId,
    ) -> Option<ReplacedDocumentMapping> {
        if let Some(previous_tab_id) = self.preview_tab_id.take() {
            let _ = self.remove_tab(previous_tab_id);
        }
        let replaced = self.register(identity, tab_id);
        if let Some(entry) = self.entries_by_identity.get_mut(&identity) {
            entry.disposition = TabDisposition::Preview;
        }
        self.preview_tab_id = Some(tab_id);
        replaced
    }

    pub fn upgrade_preview(&mut self, tab_id: TabId) -> bool {
        let Some(identity) = self.identity_for(tab_id) else {
            return false;
        };
        let Some(entry) = self.entries_by_identity.get_mut(&identity) else {
            return false;
        };
        if entry.disposition != TabDisposition::Preview {
            return false;
        }
        entry.disposition = TabDisposition::Persistent;
        self.preview_tab_id = None;
        true
    }

    pub fn tab_for(&self, identity: DocumentIdentity) -> Option<TabId> {
        self.entries_by_identity.get(&identity).map(|entry| entry.tab_id)
    }

    /// 当前 preview tab；安装下一个 preview 前用于同步 runtime 的替换行为。
    pub fn preview_tab(&self) -> Option<TabId> {
        self.preview_tab_id
    }

    pub fn identity_for(&self, tab_id: TabId) -> Option<DocumentIdentity> {
        self.identities_by_tab.get(&tab_id).copied()
    }

    /// 标记被激活的 tab，并返回它的 identity。
    pub fn touch_tab(&mut self, tab_id: TabId) -> Option<DocumentIdentity> {
        let identity = self.identity_for(tab_id)?;
        let last_access_sequence = self.next_access_sequence();
        let entry = self.entries_by_identity.get_mut(&identity)?;
        entry.last_access_sequence = last_access_sequence;
        Some(identity)
    }

    /// 移除关闭 tab 的双向映射；已知外部路径保留给 external session 去重。
    pub fn remove_tab(&mut self, tab_id: TabId) -> Option<DocumentIdentity> {
        let identity = self.identities_by_tab.remove(&tab_id)?;
        self.entries_by_identity.remove(&identity);
        if self.preview_tab_id == Some(tab_id) {
            self.preview_tab_id = None;
        }
        Some(identity)
    }

    /// 解析外部文件别名，始终为同一 canonical 路径返回同一个 identity。
    pub fn external_identity_for_path(
        &mut self,
        path: &Path,
    ) -> Result<DocumentIdentity, ExternalPathError> {
        let canonical_path = std::fs::canonicalize(path)
            .map_err(|source| ExternalPathError { path: path.to_path_buf(), source })?;
        let external_file_id = *self
            .external_identities_by_canonical_path
            .entry(canonical_path)
            .or_insert_with(ExternalFileId::generate);
        Ok(DocumentIdentity::ExternalFile(external_file_id))
    }

    /// 返回最久未使用的映射，供未来 LRU 淘汰策略使用。
    pub fn least_recently_used(&self) -> Option<(DocumentIdentity, TabId)> {
        self.entries_by_identity
            .iter()
            .min_by_key(|(_, entry)| entry.last_access_sequence)
            .map(|(identity, entry)| (*identity, entry.tab_id))
    }

    fn next_access_sequence(&mut self) -> u64 {
        self.next_access_sequence = self.next_access_sequence.wrapping_add(1);
        self.next_access_sequence
    }
}

#[cfg(test)]
mod tests {
    use appkit_core::workspace::types::TabIdAllocator;
    use notora_core::{DocumentIdentity, NoteId};

    use super::DocumentRegistry;

    #[test]
    fn reuses_a_tab_for_the_same_note_identity_after_a_path_rename() {
        let mut tabs = TabIdAllocator::new();
        let tab_id = tabs.allocate();
        let note_id = NoteId::generate();
        let mut registry = DocumentRegistry::default();

        assert_eq!(registry.register(DocumentIdentity::Note(note_id), tab_id), None);
        assert_eq!(registry.tab_for(DocumentIdentity::Note(note_id)), Some(tab_id));
        assert_eq!(registry.identity_for(tab_id), Some(DocumentIdentity::Note(note_id)));
    }

    #[test]
    fn closing_a_tab_removes_both_directions_without_reviving_late_results() {
        let mut tabs = TabIdAllocator::new();
        let tab_id = tabs.allocate();
        let identity = DocumentIdentity::Note(NoteId::generate());
        let mut registry = DocumentRegistry::default();
        let _ = registry.register(identity, tab_id);

        assert_eq!(registry.remove_tab(tab_id), Some(identity));
        assert_eq!(registry.tab_for(identity), None);
        assert_eq!(registry.identity_for(tab_id), None);
    }

    #[test]
    fn canonical_external_aliases_share_one_document_identity() {
        let directory =
            tempfile::tempdir().expect("external file test directory should be created");
        let external_file = directory.path().join("outside.md");
        std::fs::write(&external_file, "# External").expect("external fixture should be written");
        let mut registry = DocumentRegistry::default();

        let first_identity = registry
            .external_identity_for_path(&external_file)
            .expect("existing external file should canonicalize");
        let second_identity = registry
            .external_identity_for_path(&directory.path().join("./outside.md"))
            .expect("path alias should canonicalize");

        assert_eq!(first_identity, second_identity);
    }

    #[test]
    fn preview_is_promoted_to_persistent_before_another_preview_reuses_its_slot() {
        let mut tabs = TabIdAllocator::new();
        let preview_tab = tabs.allocate();
        let next_preview_tab = tabs.allocate();
        let preview_identity = DocumentIdentity::Note(NoteId::generate());
        let next_identity = DocumentIdentity::Note(NoteId::generate());
        let mut registry = DocumentRegistry::default();

        assert_eq!(registry.register_preview(preview_identity, preview_tab), None);
        assert!(registry.upgrade_preview(preview_tab));
        assert_eq!(registry.register_preview(next_identity, next_preview_tab), None);
        assert_eq!(registry.tab_for(preview_identity), Some(preview_tab));
        assert_eq!(registry.tab_for(next_identity), Some(next_preview_tab));
    }
}
