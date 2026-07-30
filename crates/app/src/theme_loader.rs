use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use ui::theme::ThemeSource;

/// Summary of everything that happened while loading and registering user themes.
#[derive(Debug, Default)]
pub(crate) struct ThemeLoadReport {
    pub(crate) source_diagnostics: Vec<ThemeSourceDiagnostic>,
    pub(crate) registry_errors: Vec<ui::theme::ThemeLoadError>,
    pub(crate) registered_ids: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct ThemeSourceBatch {
    pub(crate) sources: Vec<ThemeSource>,
    pub(crate) diagnostics: Vec<ThemeSourceDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ThemeSourceDiagnostic {
    DirectoryRead { path: PathBuf, message: String },
    EntryRead { directory: PathBuf, message: String },
    InvalidFileName { path: PathBuf },
    FileRead { path: PathBuf, message: String },
}

impl ThemeSourceDiagnostic {
    fn sort_key(&self) -> (u8, PathBuf, String) {
        match self {
            Self::DirectoryRead { path, message } => (0, path.clone(), message.clone()),
            Self::EntryRead { directory, message } => (1, directory.clone(), message.clone()),
            Self::InvalidFileName { path } => (2, path.clone(), String::new()),
            Self::FileRead { path, message } => (3, path.clone(), message.clone()),
        }
    }
}

fn collect_entry_paths<I>(
    dir: &Path,
    entries: I,
    diagnostics: &mut Vec<ThemeSourceDiagnostic>,
) -> Vec<PathBuf>
where
    I: IntoIterator<Item = io::Result<fs::DirEntry>>,
{
    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => match entry.file_type() {
                Ok(kind)
                    if kind.is_file()
                        && entry.path().extension().is_some_and(|ext| ext == "toml") =>
                {
                    paths.push(entry.path());
                }
                Ok(_) => {}
                Err(error) => diagnostics.push(ThemeSourceDiagnostic::EntryRead {
                    directory: dir.to_owned(),
                    message: error.to_string(),
                }),
            },
            Err(error) => diagnostics.push(ThemeSourceDiagnostic::EntryRead {
                directory: dir.to_owned(),
                message: error.to_string(),
            }),
        }
    }
    paths.sort();
    paths
}

pub(crate) fn load_theme_sources(dir: &Path) -> ThemeSourceBatch {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return ThemeSourceBatch::default();
        }
        Err(error) => {
            return ThemeSourceBatch {
                sources: Vec::new(),
                diagnostics: vec![ThemeSourceDiagnostic::DirectoryRead {
                    path: dir.to_owned(),
                    message: error.to_string(),
                }],
            };
        }
    };

    let mut batch = ThemeSourceBatch::default();
    let paths = collect_entry_paths(dir, entries, &mut batch.diagnostics);
    for path in paths {
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()).map(str::to_owned) else {
            batch.diagnostics.push(ThemeSourceDiagnostic::InvalidFileName { path });
            continue;
        };
        match fs::read_to_string(&path) {
            Ok(content) => batch.sources.push(ThemeSource { id, path, content }),
            Err(error) => batch
                .diagnostics
                .push(ThemeSourceDiagnostic::FileRead { path, message: error.to_string() }),
        }
    }
    batch.diagnostics.sort_by_key(ThemeSourceDiagnostic::sort_key);
    batch
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn loads_sorted_sources_and_keeps_reserved_ids() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("z.toml"), "is_dark = true\n").unwrap();
        fs::write(dir.path().join("a.toml"), "is_dark = false\n").unwrap();
        fs::write(dir.path().join("default-dark.toml"), "is_dark = true\n").unwrap();
        fs::write(dir.path().join("ignore.txt"), "ignored").unwrap();

        let batch = load_theme_sources(dir.path());
        assert!(batch.diagnostics.is_empty());
        assert_eq!(
            batch.sources.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "default-dark", "z"]
        );
    }

    #[test]
    fn invalid_utf8_file_does_not_block_valid_source() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a-bad.toml"), [0xff, 0xfe]).unwrap();
        fs::write(dir.path().join("z-good.toml"), "is_dark = true\n").unwrap();

        let batch = load_theme_sources(dir.path());
        assert_eq!(batch.sources.len(), 1);
        assert_eq!(batch.sources[0].id, "z-good");
        assert!(matches!(
            batch.diagnostics.as_slice(),
            [ThemeSourceDiagnostic::FileRead { path, .. }]
                if path.ends_with("a-bad.toml")
        ));
    }

    #[test]
    fn missing_directory_is_empty_without_diagnostic() {
        let dir = tempdir().unwrap();
        let batch = load_theme_sources(&dir.path().join("missing"));
        assert!(batch.sources.is_empty());
        assert!(batch.diagnostics.is_empty());
    }

    #[test]
    fn non_directory_path_reports_directory_read() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("not-a-directory");
        fs::write(&file, "content").unwrap();
        let batch = load_theme_sources(&file);
        assert!(batch.sources.is_empty());
        assert!(matches!(
            batch.diagnostics.as_slice(),
            [ThemeSourceDiagnostic::DirectoryRead { path, .. }] if path == &file
        ));
    }

    #[cfg(unix)]
    #[test]
    fn invalid_utf8_file_name_is_reported_without_stopping() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let dir = tempdir().unwrap();
        let invalid = dir.path().join(OsString::from_vec(vec![0xff, b'.', b't', b'o', b'm', b'l']));
        if fs::write(&invalid, "is_dark = true\n").is_err() {
            // Filesystem does not support invalid-UTF8 filenames (e.g. macOS APFS/HFS+).
            return;
        }
        fs::write(dir.path().join("valid.toml"), "is_dark = true\n").unwrap();
        let batch = load_theme_sources(dir.path());
        assert_eq!(
            batch.sources.iter().map(|source| source.id.as_str()).collect::<Vec<_>>(),
            vec!["valid"]
        );
        assert!(batch.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            ThemeSourceDiagnostic::InvalidFileName { path } if path == &invalid
        )));
    }

    #[test]
    fn entry_error_is_retained() {
        let dir = Path::new("themes");
        let mut diagnostics = Vec::new();
        let paths = collect_entry_paths(
            dir,
            vec![Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))],
            &mut diagnostics,
        );
        assert!(paths.is_empty());
        assert!(matches!(
            diagnostics.as_slice(),
            [ThemeSourceDiagnostic::EntryRead { directory, message }]
                if directory == dir && message.contains("denied")
        ));
    }
}
