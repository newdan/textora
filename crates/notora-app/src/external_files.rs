//! 不进入工作区 catalog 的外部文件会话。

use std::path::{Path, PathBuf};

use notora_core::{DocumentIdentity, DocumentKind, ExternalFileId};

/// 已被规范化的外部文件绝对路径。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CanonicalExternalPath(PathBuf);

impl CanonicalExternalPath {
    /// 在产品 I/O 边界规范化一个已有文件路径。
    pub fn canonicalize(path: &Path) -> Result<Self, ExternalFilePathError> {
        let canonical_path = std::fs::canonicalize(path).map_err(|source| {
            ExternalFilePathError::Canonicalize { path: path.to_path_buf(), source }
        })?;
        let metadata = std::fs::metadata(&canonical_path).map_err(|source| {
            ExternalFilePathError::Metadata { path: canonical_path.clone(), source }
        })?;
        if !metadata.is_file() {
            return Err(ExternalFilePathError::NotAFile { path: canonical_path });
        }
        Ok(Self(canonical_path))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// 可由 editor plugin 路由的外部 UTF-8 文本文档。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedExternalTextFile {
    pub canonical_path: CanonicalExternalPath,
    pub kind: DocumentKind,
}

/// 验证外部文件是否能进入 notora 的文本编辑会话。
pub fn validate_external_text_file(
    path: &Path,
) -> Result<ValidatedExternalTextFile, ExternalFileOpenError> {
    let canonical_path =
        CanonicalExternalPath::canonicalize(path).map_err(ExternalFileOpenError::Path)?;
    let kind = DocumentKind::from_external_path(canonical_path.as_path()).ok_or_else(|| {
        ExternalFileOpenError::UnsupportedKind { path: canonical_path.as_path().to_path_buf() }
    })?;
    let bytes = std::fs::read(canonical_path.as_path()).map_err(|source| {
        ExternalFileOpenError::Read { path: canonical_path.as_path().to_path_buf(), source }
    })?;
    if bytes.contains(&0) {
        return Err(ExternalFileOpenError::BinaryContent {
            path: canonical_path.as_path().to_path_buf(),
        });
    }
    std::str::from_utf8(&bytes).map_err(|_| ExternalFileOpenError::InvalidUtf8 {
        path: canonical_path.as_path().to_path_buf(),
    })?;
    Ok(ValidatedExternalTextFile { canonical_path, kind })
}

/// 外部文件无法作为 text session 打开。
#[derive(Debug)]
pub enum ExternalFileOpenError {
    Path(ExternalFilePathError),
    UnsupportedKind { path: PathBuf },
    Read { path: PathBuf, source: std::io::Error },
    BinaryContent { path: PathBuf },
    InvalidUtf8 { path: PathBuf },
}

impl std::fmt::Display for ExternalFileOpenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path(error) => error.fmt(formatter),
            Self::UnsupportedKind { path } => write!(
                formatter,
                "unsupported external file type for {}; expected a common UTF-8 text format",
                path.display()
            ),
            Self::Read { path, source } => {
                write!(formatter, "could not read external file {}: {source}", path.display())
            }
            Self::BinaryContent { path } => {
                write!(formatter, "external file contains binary data: {}", path.display())
            }
            Self::InvalidUtf8 { path } => {
                write!(formatter, "external file is not valid UTF-8: {}", path.display())
            }
        }
    }
}

impl std::error::Error for ExternalFileOpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Path(error) => Some(error),
            Self::Read { source, .. } => Some(source),
            Self::UnsupportedKind { .. }
            | Self::BinaryContent { .. }
            | Self::InvalidUtf8 { .. } => None,
        }
    }
}

/// 外部文件路径不能被作为已有文件打开。
#[derive(Debug)]
pub enum ExternalFilePathError {
    Canonicalize { path: PathBuf, source: std::io::Error },
    Metadata { path: PathBuf, source: std::io::Error },
    NotAFile { path: PathBuf },
}

impl std::fmt::Display for ExternalFilePathError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Canonicalize { path, source } => {
                write!(
                    formatter,
                    "could not canonicalize external file {}: {source}",
                    path.display()
                )
            }
            Self::Metadata { path, source } => {
                write!(formatter, "could not inspect external file {}: {source}", path.display())
            }
            Self::NotAFile { path } => {
                write!(formatter, "external path is not a file: {}", path.display())
            }
        }
    }
}

impl std::error::Error for ExternalFilePathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Canonicalize { source, .. } | Self::Metadata { source, .. } => Some(source),
            Self::NotAFile { .. } => None,
        }
    }
}

/// 外部文件 session 的互斥磁盘状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalFileSession {
    Existing { external_file_id: ExternalFileId, canonical_path: CanonicalExternalPath },
    Untitled { external_file_id: ExternalFileId, kind: DocumentKind },
    Missing { external_file_id: ExternalFileId, last_known_path: PathBuf },
}

impl ExternalFileSession {
    pub fn external_file_id(&self) -> ExternalFileId {
        match self {
            Self::Existing { external_file_id, .. }
            | Self::Untitled { external_file_id, .. }
            | Self::Missing { external_file_id, .. } => *external_file_id,
        }
    }

    pub fn identity(&self) -> DocumentIdentity {
        DocumentIdentity::ExternalFile(self.external_file_id())
    }
}

/// 打开已有路径时是否复用了已有 session。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenExistingExternalFile {
    Created(DocumentIdentity),
    Reused(DocumentIdentity),
}

impl OpenExistingExternalFile {
    pub fn identity(self) -> DocumentIdentity {
        match self {
            Self::Created(identity) | Self::Reused(identity) => identity,
        }
    }
}

/// missing session 重新定位后的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelocateExternalFile {
    Relocated(DocumentIdentity),
    ReusedExisting(DocumentIdentity),
}

/// Save As 对外部 session 的结果；已经打开的目标路径不会吞掉当前 session。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveExternalFileAs {
    Updated(DocumentIdentity),
    PathAlreadyOpen(DocumentIdentity),
}

impl SaveExternalFileAs {
    pub fn identity(self) -> DocumentIdentity {
        match self {
            Self::Updated(identity) | Self::PathAlreadyOpen(identity) => identity,
        }
    }
}

impl RelocateExternalFile {
    pub fn identity(self) -> DocumentIdentity {
        match self {
            Self::Relocated(identity) | Self::ReusedExisting(identity) => identity,
        }
    }
}

/// 不依赖 catalog 的外部文件 session 集合，按打开顺序保存。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternalFileSessions {
    sessions: Vec<ExternalFileSession>,
}

impl ExternalFileSessions {
    /// 同一路径只创建一个 session；调用者必须先完成路径 canonicalize。
    pub fn open_existing(
        &mut self,
        canonical_path: CanonicalExternalPath,
    ) -> OpenExistingExternalFile {
        if let Some(session) = self.sessions.iter().find(|session| {
            matches!(session, ExternalFileSession::Existing { canonical_path: known_path, .. } if known_path == &canonical_path)
        }) {
            return OpenExistingExternalFile::Reused(session.identity());
        }
        let external_file_id = ExternalFileId::generate();
        let identity = DocumentIdentity::ExternalFile(external_file_id);
        self.sessions.push(ExternalFileSession::Existing { external_file_id, canonical_path });
        OpenExistingExternalFile::Created(identity)
    }

    pub fn create_untitled(&mut self, kind: DocumentKind) -> DocumentIdentity {
        let external_file_id = ExternalFileId::generate();
        let identity = DocumentIdentity::ExternalFile(external_file_id);
        self.sessions.push(ExternalFileSession::Untitled { external_file_id, kind });
        identity
    }

    /// 将已有文件转为可恢复的 missing session；未保存的 untitled 不会变为 missing。
    pub fn mark_missing(&mut self, external_file_id: ExternalFileId) -> bool {
        let Some(index) = self.index_for(external_file_id) else {
            return false;
        };
        let ExternalFileSession::Existing { canonical_path, .. } = &self.sessions[index] else {
            return false;
        };
        self.sessions[index] = ExternalFileSession::Missing {
            external_file_id,
            last_known_path: canonical_path.as_path().to_path_buf(),
        };
        true
    }

    /// 重新定位 missing session；若目标已有 session，则删除这个 missing session 并复用目标。
    pub fn relocate_missing(
        &mut self,
        external_file_id: ExternalFileId,
        canonical_path: CanonicalExternalPath,
    ) -> Option<RelocateExternalFile> {
        let missing_index = self.index_for(external_file_id)?;
        if !matches!(self.sessions[missing_index], ExternalFileSession::Missing { .. }) {
            return None;
        }
        if let Some(existing_identity) = self.sessions.iter().find_map(|session| match session {
            ExternalFileSession::Existing { canonical_path: known_path, .. }
                if known_path == &canonical_path =>
            {
                Some(session.identity())
            }
            _ => None,
        }) {
            self.sessions.remove(missing_index);
            return Some(RelocateExternalFile::ReusedExisting(existing_identity));
        }
        let identity = DocumentIdentity::ExternalFile(external_file_id);
        self.sessions[missing_index] =
            ExternalFileSession::Existing { external_file_id, canonical_path };
        Some(RelocateExternalFile::Relocated(identity))
    }

    /// Save As 成功后保留原有 external identity，并以规范化目标路径替换 session 状态。
    ///
    /// 若该路径已由另一个 session 打开，则保持原 session 不变，交由产品层提示用户。
    pub fn save_as(
        &mut self,
        external_file_id: ExternalFileId,
        canonical_path: CanonicalExternalPath,
    ) -> Option<SaveExternalFileAs> {
        let session_index = self.index_for(external_file_id)?;
        if let Some(existing_identity) = self.sessions.iter().find_map(|session| match session {
            ExternalFileSession::Existing { canonical_path: known_path, .. }
                if known_path == &canonical_path
                    && session.external_file_id() != external_file_id =>
            {
                Some(session.identity())
            }
            _ => None,
        }) {
            return Some(SaveExternalFileAs::PathAlreadyOpen(existing_identity));
        }
        let identity = DocumentIdentity::ExternalFile(external_file_id);
        self.sessions[session_index] =
            ExternalFileSession::Existing { external_file_id, canonical_path };
        Some(SaveExternalFileAs::Updated(identity))
    }

    /// 仅移除产品 session；不会删除或移动磁盘文件。
    pub fn remove(&mut self, external_file_id: ExternalFileId) -> Option<ExternalFileSession> {
        let index = self.index_for(external_file_id)?;
        Some(self.sessions.remove(index))
    }

    pub fn session(&self, external_file_id: ExternalFileId) -> Option<&ExternalFileSession> {
        self.sessions.iter().find(|session| session.external_file_id() == external_file_id)
    }

    /// 根据规范化路径读取已有会话身份，供 session 恢复精确选回最后一个 external 文档。
    pub fn identity_for_canonical_path(
        &self,
        canonical_path: &CanonicalExternalPath,
    ) -> Option<DocumentIdentity> {
        self.sessions.iter().find_map(|session| match session {
            ExternalFileSession::Existing { canonical_path: known_path, .. }
                if known_path == canonical_path =>
            {
                Some(session.identity())
            }
            ExternalFileSession::Existing { .. }
            | ExternalFileSession::Untitled { .. }
            | ExternalFileSession::Missing { .. } => None,
        })
    }

    pub fn sessions(&self) -> &[ExternalFileSession] {
        &self.sessions
    }

    fn index_for(&self, external_file_id: ExternalFileId) -> Option<usize> {
        self.sessions.iter().position(|session| session.external_file_id() == external_file_id)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use notora_core::{DocumentIdentity, DocumentKind};

    use super::{
        CanonicalExternalPath, ExternalFileOpenError, ExternalFileSession, ExternalFileSessions,
        OpenExistingExternalFile, RelocateExternalFile, SaveExternalFileAs,
        validate_external_text_file,
    };

    fn canonical_fixture(name: &str) -> CanonicalExternalPath {
        let directory =
            tempfile::tempdir().expect("external session fixture directory should exist");
        let path = directory.keep().join(name);
        fs::write(&path, "# External").expect("external session fixture should be written");
        CanonicalExternalPath::canonicalize(&path).expect("fixture file should canonicalize")
    }

    fn external_file_id(identity: DocumentIdentity) -> notora_core::ExternalFileId {
        match identity {
            DocumentIdentity::ExternalFile(external_file_id) => external_file_id,
            DocumentIdentity::Note(_) => panic!("external session must use an external identity"),
        }
    }

    #[test]
    fn opening_the_same_canonical_path_reuses_its_external_identity() {
        let path = canonical_fixture("same.md");
        let mut sessions = ExternalFileSessions::default();

        let first = sessions.open_existing(path.clone());
        let second = sessions.open_existing(path);

        assert!(matches!(first, OpenExistingExternalFile::Created(_)));
        assert_eq!(second, OpenExistingExternalFile::Reused(first.identity()));
        assert_eq!(sessions.sessions().len(), 1);
    }

    #[test]
    fn session_restore_can_find_an_existing_external_document_by_its_canonical_path() {
        let path = canonical_fixture("restore-target.md");
        let mut sessions = ExternalFileSessions::default();
        let identity = sessions.open_existing(path.clone()).identity();

        assert_eq!(sessions.identity_for_canonical_path(&path), Some(identity));
    }

    #[test]
    fn sessions_keep_existing_untitled_and_missing_states_mutually_exclusive() {
        let path = canonical_fixture("missing.md");
        let mut sessions = ExternalFileSessions::default();
        let existing_identity = sessions.open_existing(path).identity();
        let existing_id = external_file_id(existing_identity);
        let untitled_identity = sessions.create_untitled(DocumentKind::Markdown);

        assert!(sessions.mark_missing(existing_id));
        assert!(matches!(sessions.session(existing_id), Some(ExternalFileSession::Missing { .. })));
        assert!(matches!(
            sessions.session(external_file_id(untitled_identity)),
            Some(ExternalFileSession::Untitled { kind: DocumentKind::Markdown, .. })
        ));
    }

    #[test]
    fn relocating_missing_entry_reuses_an_existing_path_session() {
        let original_path = canonical_fixture("original.md");
        let shared_path = canonical_fixture("shared.md");
        let mut sessions = ExternalFileSessions::default();
        let missing_identity = sessions.open_existing(original_path).identity();
        let missing_id = external_file_id(missing_identity);
        let existing_identity = sessions.open_existing(shared_path.clone()).identity();
        assert!(sessions.mark_missing(missing_id));

        assert_eq!(
            sessions.relocate_missing(missing_id, shared_path),
            Some(RelocateExternalFile::ReusedExisting(existing_identity))
        );
        assert_eq!(sessions.sessions().len(), 1);
        assert_eq!(sessions.session(missing_id), None);
    }

    #[test]
    fn relocating_missing_entry_keeps_its_original_external_identity() {
        let original_path = canonical_fixture("old-location.md");
        let relocated_path = canonical_fixture("new-location.md");
        let mut sessions = ExternalFileSessions::default();
        let identity = sessions.open_existing(original_path).identity();
        let external_file_id = external_file_id(identity);
        assert!(sessions.mark_missing(external_file_id));

        assert_eq!(
            sessions.relocate_missing(external_file_id, relocated_path.clone()),
            Some(RelocateExternalFile::Relocated(identity))
        );
        assert!(matches!(
            sessions.session(external_file_id),
            Some(ExternalFileSession::Existing { canonical_path, .. })
                if canonical_path == &relocated_path
        ));
    }

    #[test]
    fn save_as_keeps_an_untitled_external_identity_and_updates_its_canonical_path() {
        let path = canonical_fixture("saved-from-untitled.md");
        let mut sessions = ExternalFileSessions::default();
        let identity = sessions.create_untitled(DocumentKind::Markdown);
        let external_file_id = external_file_id(identity);

        assert_eq!(
            sessions.save_as(external_file_id, path.clone()),
            Some(SaveExternalFileAs::Updated(identity))
        );
        assert!(matches!(
            sessions.session(external_file_id),
            Some(ExternalFileSession::Existing { canonical_path, .. }) if canonical_path == &path
        ));
    }

    #[test]
    fn save_as_rejects_a_path_owned_by_another_external_session_without_mutating_either() {
        let first_path = canonical_fixture("first.md");
        let mut sessions = ExternalFileSessions::default();
        let existing_identity = sessions.open_existing(first_path.clone()).identity();
        let untitled_identity = sessions.create_untitled(DocumentKind::Markdown);
        let untitled_id = external_file_id(untitled_identity);

        assert_eq!(
            sessions.save_as(untitled_id, first_path),
            Some(SaveExternalFileAs::PathAlreadyOpen(existing_identity))
        );
        assert!(matches!(
            sessions.session(untitled_id),
            Some(ExternalFileSession::Untitled { .. })
        ));
    }

    #[test]
    fn removing_a_session_never_deletes_its_external_file() {
        let directory =
            tempfile::tempdir().expect("external session fixture directory should exist");
        let path = directory.path().join("keep.md");
        fs::write(&path, "# Keep").expect("external fixture should be written");
        let canonical_path =
            CanonicalExternalPath::canonicalize(&path).expect("fixture file should canonicalize");
        let mut sessions = ExternalFileSessions::default();
        let identity = sessions.open_existing(canonical_path).identity();
        let external_file_id = external_file_id(identity);

        let removed = sessions.remove(external_file_id);

        assert!(matches!(removed, Some(ExternalFileSession::Existing { .. })));
        assert!(path.is_file());
    }

    #[test]
    fn validation_rejects_binary_unsupported_and_invalid_utf8_files() {
        let directory = tempfile::tempdir().expect("external validation directory should exist");
        let binary_path = directory.path().join("binary.md");
        let unsupported_path = directory.path().join("image.png");
        let invalid_utf8_path = directory.path().join("invalid.txt");
        fs::write(&binary_path, [b'a', 0, b'b']).expect("binary fixture should be written");
        fs::write(&unsupported_path, "image").expect("unsupported fixture should be written");
        fs::write(&invalid_utf8_path, [0xff, 0xfe])
            .expect("invalid UTF-8 fixture should be written");

        assert!(matches!(
            validate_external_text_file(&binary_path),
            Err(ExternalFileOpenError::BinaryContent { .. })
        ));
        assert!(matches!(
            validate_external_text_file(&unsupported_path),
            Err(ExternalFileOpenError::UnsupportedKind { .. })
        ));
        assert!(matches!(
            validate_external_text_file(&invalid_utf8_path),
            Err(ExternalFileOpenError::InvalidUtf8 { .. })
        ));
    }

    #[test]
    fn validation_accepts_common_utf8_text_formats() {
        let directory = tempfile::tempdir().expect("external validation directory should exist");
        for file_name in ["settings.json", "notes.yaml", "Cargo.toml", "records.csv", "main.rs"] {
            let path = directory.path().join(file_name);
            fs::write(&path, "text content").expect("text fixture should be written");

            let validated = validate_external_text_file(&path)
                .expect("common UTF-8 text format should be accepted");

            assert_eq!(validated.kind, DocumentKind::Text);
        }
    }
}
