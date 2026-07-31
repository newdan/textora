use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 稳定的工作区身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceId(Uuid);

impl WorkspaceId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl std::fmt::Display for WorkspaceId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// 稳定的笔记身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NoteId(Uuid);

impl NoteId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl From<Uuid> for NoteId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for NoteId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// 稳定的标签身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TagId(Uuid);

impl TagId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl std::fmt::Display for TagId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// 产品 session 中外部文件的身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExternalFileId(Uuid);

impl ExternalFileId {
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl std::fmt::Display for ExternalFileId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// notora 首版支持的文本文件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DocumentKind {
    Text,
    Markdown,
    Mindmap,
}

impl DocumentKind {
    /// 根据完整文件名和扩展名识别可编辑文本类型。
    ///
    /// `.mmap.md` 必须先于通用 `.md` 匹配，以保证 Mindmap 路由稳定。
    pub fn from_path(path: &Path) -> Option<Self> {
        let file_name = path.file_name()?.to_str()?;
        if file_name.ends_with(".mmap.md") {
            return Some(Self::Mindmap);
        }

        match path.extension()?.to_str()? {
            "md" => Some(Self::Markdown),
            "txt" => Some(Self::Text),
            _ => None,
        }
    }
}

/// 编辑文档的磁盘来源，决定保存策略。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentOrigin {
    Note { workspace_id: WorkspaceId, note_id: NoteId, relative_path: PathBuf },
    ExternalFile { external_file_id: ExternalFileId, canonical_path: PathBuf },
    UntitledExternal { external_file_id: ExternalFileId, kind: DocumentKind },
}

impl DocumentOrigin {
    pub fn identity(&self) -> DocumentIdentity {
        match self {
            Self::Note { note_id, .. } => DocumentIdentity::Note(*note_id),
            Self::ExternalFile { external_file_id, .. }
            | Self::UntitledExternal { external_file_id, .. } => {
                DocumentIdentity::ExternalFile(*external_file_id)
            }
        }
    }
}

/// 产品层到 `TabId` 的稳定映射键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocumentIdentity {
    Note(NoteId),
    ExternalFile(ExternalFileId),
}

/// 笔记在 catalog 中的生命周期。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteLifecycle {
    Active,
    Trashed { original_relative_path: PathBuf, deleted_at: SystemTime },
}

/// 卡片中显示的标签摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSummary {
    pub tag_id: TagId,
    pub display_name: String,
}

/// 中栏卡片和查询结果所需的预计算摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSummary {
    pub note_id: NoteId,
    pub relative_path: PathBuf,
    pub kind: DocumentKind,
    pub title: String,
    pub excerpt: String,
    pub modified_at: SystemTime,
    pub starred: bool,
    pub tags: Vec<TagSummary>,
    pub lifecycle: NoteLifecycle,
}

/// 左侧导航和中栏查询的领域范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationScope {
    Search { query: String },
    WorkspaceRoot,
    Directory { relative_path: PathBuf },
    Starred,
    Trash,
    Tag { tag_id: TagId },
    ExternalFiles,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{DocumentIdentity, DocumentKind, DocumentOrigin, ExternalFileId, NoteId};

    #[test]
    fn mindmap_suffix_has_priority_over_markdown_extension() {
        assert_eq!(
            DocumentKind::from_path(Path::new("architecture.mmap.md")),
            Some(DocumentKind::Mindmap)
        );
    }

    #[test]
    fn supported_extensions_map_to_expected_kinds() {
        assert_eq!(DocumentKind::from_path(Path::new("notes.md")), Some(DocumentKind::Markdown));
        assert_eq!(DocumentKind::from_path(Path::new("draft.txt")), Some(DocumentKind::Text));
        assert_eq!(DocumentKind::from_path(Path::new("asset.png")), None);
    }

    #[test]
    fn note_origin_uses_note_identity_after_path_changes() {
        let note_id = NoteId::generate();
        let origin = DocumentOrigin::Note {
            workspace_id: super::WorkspaceId::generate(),
            note_id,
            relative_path: "ideas/first.md".into(),
        };

        assert_eq!(origin.identity(), DocumentIdentity::Note(note_id));
    }

    #[test]
    fn untitled_and_saved_external_documents_share_an_external_identity() {
        let external_file_id = ExternalFileId::generate();
        let untitled =
            DocumentOrigin::UntitledExternal { external_file_id, kind: DocumentKind::Text };
        let saved = DocumentOrigin::ExternalFile {
            external_file_id,
            canonical_path: "/tmp/external.txt".into(),
        };

        assert_eq!(untitled.identity(), saved.identity());
    }
}
