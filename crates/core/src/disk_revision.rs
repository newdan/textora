use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::file::FileError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiskRevision {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub content_hash: blake3::Hash,
    pub file_identity: Option<FileIdentity>,
}

pub fn read_disk_revision(path: &Path) -> Result<Option<DiskRevision>, FileError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(FileError::Io(error)),
    };
    if !metadata.is_file() {
        return Err(FileError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path is not a regular file",
        )));
    }

    let mut file = fs::File::open(path).map_err(FileError::Io)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let bytes_read = file.read(&mut buffer).map_err(FileError::Io)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(Some(DiskRevision {
        path: path.to_owned(),
        size: metadata.len(),
        modified: metadata.modified().ok(),
        content_hash: hasher.finalize(),
        file_identity: file_identity(&metadata),
    }))
}

#[cfg(unix)]
fn file_identity(metadata: &fs::Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    Some(FileIdentity { device: metadata.dev(), inode: metadata.ino() })
}

#[cfg(not(unix))]
fn file_identity(_metadata: &fs::Metadata) -> Option<FileIdentity> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn same_content_and_metadata_have_equal_revisions() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        fs::write(&path, b"same").expect("file should be written");

        let first = read_disk_revision(&path)
            .expect("revision should be readable")
            .expect("file should exist");
        let second = read_disk_revision(&path)
            .expect("revision should be readable")
            .expect("file should exist");

        assert_eq!(first, second);
    }

    #[test]
    fn same_size_and_mtime_with_changed_content_have_different_hashes() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        let fixed_time = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut file = fs::File::create(&path).expect("file should be created");
        file.write_all(b"old!").expect("initial content should be written");
        file.set_modified(fixed_time).expect("mtime should be set");
        drop(file);
        let first = read_disk_revision(&path)
            .expect("revision should be readable")
            .expect("file should exist");

        let mut file = OpenOptions::new().write(true).open(&path).expect("file should open");
        file.write_all(b"new!").expect("replacement content should be written");
        file.set_len(4).expect("file size should remain unchanged");
        file.set_modified(fixed_time).expect("mtime should be restored");
        drop(file);
        let second = read_disk_revision(&path)
            .expect("revision should be readable")
            .expect("file should exist");

        assert_eq!(first.size, second.size);
        assert_eq!(first.modified, second.modified);
        assert_ne!(first.content_hash, second.content_hash);
    }

    #[test]
    fn replacing_inode_changes_file_identity() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        let replacement = directory.path().join("replacement.tmp");
        fs::write(&path, b"content").expect("file should be written");
        let first = read_disk_revision(&path)
            .expect("revision should be readable")
            .expect("file should exist");

        fs::write(&replacement, b"content").expect("replacement should be written");
        fs::rename(&replacement, &path).expect("replacement should be renamed");
        let second = read_disk_revision(&path)
            .expect("revision should be readable")
            .expect("file should exist");

        if first.file_identity.is_some() && second.file_identity.is_some() {
            assert_ne!(first.file_identity, second.file_identity);
        }
    }

    #[test]
    fn missing_path_returns_none_and_directory_is_rejected() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let missing = directory.path().join("missing.md");

        assert_eq!(read_disk_revision(&missing).expect("missing path is not an error"), None);
        assert!(read_disk_revision(directory.path()).is_err());
    }
}
