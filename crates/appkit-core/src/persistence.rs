use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
    }

    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let pid = std::process::id();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_name = format!(".{}.{}.{}.tmp", file_name, pid, count);

    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let temp_path = parent.join(&temp_name);

    struct TempFileGuard {
        path: PathBuf,
        delete: bool,
    }

    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            if self.delete {
                let _ = fs::remove_file(&self.path);
            }
        }
    }

    let mut guard = TempFileGuard { path: temp_path.clone(), delete: true };

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    let mut temp_file = options.open(&guard.path)?;

    if path.exists()
        && let Ok(metadata) = fs::metadata(path)
    {
        let _ = temp_file.set_permissions(metadata.permissions());
    }

    temp_file.write_all(bytes)?;
    temp_file.flush()?;
    temp_file.sync_all()?;

    fs::rename(&guard.path, path)?;

    guard.delete = false;

    #[cfg(unix)]
    if let Some(parent) = path.parent()
        && let Ok(dir) =
            File::open(if parent.as_os_str().is_empty() { Path::new(".") } else { parent })
    {
        let _ = dir.sync_all();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_files(dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && let Some(name) = path.file_name().and_then(|n| n.to_str())
                    && name.ends_with(".tmp")
                {
                    files.push(path);
                }
            }
        }
        files
    }

    #[test]
    fn atomic_write_replaces_existing_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        std::fs::write(&path, b"old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(temp_files(dir.path()), Vec::<PathBuf>::new());
    }

    #[test]
    fn atomic_write_creates_missing_parent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/state.toml");
        atomic_write(&path, b"state").unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"state");
    }

    #[test]
    fn failed_rename_does_not_leave_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let target_directory = dir.path().join("target");
        std::fs::create_dir(&target_directory).unwrap();
        assert!(atomic_write(&target_directory, b"state").is_err());
        assert_eq!(temp_files(dir.path()), Vec::<PathBuf>::new());
    }
}
