//! 普通工作区笔记的标题派生文件名协议。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::DocumentKind;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

pub const DEFAULT_NOTE_TITLE: &str = "无标题";
pub const MAX_AUTOMATIC_NAME_DISAMBIGUATOR: u32 = 1_000_000;
pub const MAX_NOTE_FILE_STEM_GRAPHEMES: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocatedTitleBoundFileName {
    pub file_name: String,
    pub disambiguator: u32,
}

pub fn allocate_title_bound_file_name(
    absolute_directory: &Path,
    relative_directory: &Path,
    normalized_stem: &str,
    kind: DocumentKind,
    current_relative_path: Option<&Path>,
    catalog_relative_paths: &[PathBuf],
) -> Result<Option<AllocatedTitleBoundFileName>, std::io::Error> {
    let mut occupied_keys = HashSet::new();
    for entry in std::fs::read_dir(absolute_directory)? {
        let entry = entry?;
        let relative_path = relative_directory.join(entry.file_name());
        if current_relative_path == Some(relative_path.as_path()) {
            continue;
        }
        occupied_keys.insert(file_name_collision_key(&entry.file_name().to_string_lossy()));
    }
    for relative_path in catalog_relative_paths {
        if relative_path.parent() != Some(relative_directory)
            || current_relative_path == Some(relative_path.as_path())
        {
            continue;
        }
        if let Some(file_name) = relative_path.file_name() {
            occupied_keys.insert(file_name_collision_key(&file_name.to_string_lossy()));
        }
    }

    for disambiguator in 1..=MAX_AUTOMATIC_NAME_DISAMBIGUATOR {
        let file_name = title_bound_file_name(normalized_stem, kind, disambiguator);
        if occupied_keys.contains(&file_name_collision_key(&file_name)) {
            continue;
        }
        return Ok(Some(AllocatedTitleBoundFileName { file_name, disambiguator }));
    }

    Ok(None)
}

pub fn normalize_title_file_stem(title: &str) -> String {
    let sanitized = title
        .nfc()
        .map(|character| {
            if character.is_control() || is_forbidden_file_name_character(character) {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let collapsed = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_end_matches([' ', '.']);
    let truncated = trimmed.graphemes(true).take(MAX_NOTE_FILE_STEM_GRAPHEMES).collect::<String>();
    let normalized = truncated.trim_end_matches([' ', '.']);
    if normalized.is_empty() || is_reserved_file_stem(normalized) {
        return DEFAULT_NOTE_TITLE.to_owned();
    }

    normalized.to_owned()
}

pub fn title_bound_file_name(
    normalized_stem: &str,
    kind: DocumentKind,
    disambiguator: u32,
) -> String {
    let suffix = if disambiguator <= 1 { String::new() } else { format!(" ({disambiguator})") };
    format!("{normalized_stem}{suffix}{}", document_file_extension(kind))
}

pub fn file_name_collision_key(file_name: &str) -> String {
    file_name.nfkc().flat_map(char::to_lowercase).collect::<String>().nfc().collect()
}

pub fn document_file_extension(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Text => ".txt",
        DocumentKind::Markdown => ".md",
        DocumentKind::Mindmap => ".mmap.md",
    }
}

fn is_forbidden_file_name_character(character: char) -> bool {
    matches!(character, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

fn is_reserved_file_stem(stem: &str) -> bool {
    let device_name = stem.split('.').next().unwrap_or(stem).to_ascii_uppercase();
    matches!(device_name.as_str(), "." | ".." | "CON" | "PRN" | "AUX" | "NUL")
        || numbered_device_name(&device_name, "COM")
        || numbered_device_name(&device_name, "LPT")
}

fn numbered_device_name(device_name: &str, prefix: &str) -> bool {
    device_name
        .strip_prefix(prefix)
        .is_some_and(|number| matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use unicode_segmentation::UnicodeSegmentation;

    use super::{
        DEFAULT_NOTE_TITLE, MAX_NOTE_FILE_STEM_GRAPHEMES, allocate_title_bound_file_name,
        file_name_collision_key, normalize_title_file_stem, title_bound_file_name,
    };
    use crate::DocumentKind;

    #[test]
    fn file_name_normalization_preserves_unicode_and_removes_unsafe_components() {
        assert_eq!(normalize_title_file_stem("  项目 / 路线:*?\"<>| \\ 图.  "), "项目 路线 图");
        assert_eq!(normalize_title_file_stem("Cafe\u{301}"), "Café");
        assert_eq!(normalize_title_file_stem("产品 🚀 计划"), "产品 🚀 计划");
    }

    #[test]
    fn empty_dot_and_reserved_device_titles_use_the_safe_default() {
        for title in ["", " \t\n", ".", "..", "CON", "com1", "Lpt9..."] {
            assert_eq!(normalize_title_file_stem(title), DEFAULT_NOTE_TITLE);
        }
    }

    #[test]
    fn normalized_stem_is_truncated_at_a_grapheme_boundary() {
        let long_title = "👨‍👩‍👧‍👦".repeat(MAX_NOTE_FILE_STEM_GRAPHEMES + 1);
        let normalized = normalize_title_file_stem(&long_title);

        assert_eq!(normalized.graphemes(true).count(), MAX_NOTE_FILE_STEM_GRAPHEMES);
        assert!(normalized.ends_with("👨‍👩‍👧‍👦"));
    }

    #[test]
    fn title_bound_names_keep_document_kind_and_stable_disambiguator() {
        assert_eq!(title_bound_file_name("项目计划", DocumentKind::Markdown, 1), "项目计划.md");
        assert_eq!(title_bound_file_name("项目计划", DocumentKind::Text, 2), "项目计划 (2).txt");
        assert_eq!(
            title_bound_file_name("项目计划", DocumentKind::Mindmap, 3),
            "项目计划 (3).mmap.md"
        );
    }

    #[test]
    fn collision_keys_fold_case_and_unicode_normalization() {
        assert_eq!(file_name_collision_key("Plan.md"), file_name_collision_key("plan.md"));
        assert_eq!(file_name_collision_key("Cafe\u{301}.md"), file_name_collision_key("Café.md"));
    }

    #[test]
    fn allocator_respects_disk_catalog_and_case_folded_occupancy() {
        let directory = tempfile::tempdir().expect("allocation directory should be created");
        fs::write(directory.path().join("Plan.md"), "file")
            .expect("file occupancy fixture should be written");
        fs::create_dir(directory.path().join("plan (2).md"))
            .expect("directory occupancy fixture should be created");
        let catalog_paths = vec![std::path::PathBuf::from("PLAN (3).md")];

        let allocated = allocate_title_bound_file_name(
            directory.path(),
            std::path::Path::new(""),
            "plan",
            DocumentKind::Markdown,
            None,
            &catalog_paths,
        )
        .expect("directory should be readable")
        .expect("an automatic name should remain available");

        assert_eq!(allocated.file_name, "plan (4).md");
        assert_eq!(allocated.disambiguator, 4);
    }

    #[test]
    fn allocator_reuses_the_current_note_and_does_not_compact_other_names() {
        let directory = tempfile::tempdir().expect("allocation directory should be created");
        fs::write(directory.path().join("项目.md"), "current")
            .expect("current note fixture should be written");
        fs::write(directory.path().join("项目 (3).md"), "other")
            .expect("numbered note fixture should be written");
        let catalog_paths =
            vec![std::path::PathBuf::from("项目.md"), std::path::PathBuf::from("项目 (3).md")];

        let current = allocate_title_bound_file_name(
            directory.path(),
            std::path::Path::new(""),
            "项目",
            DocumentKind::Markdown,
            Some(std::path::Path::new("项目.md")),
            &catalog_paths,
        )
        .expect("directory should be readable")
        .expect("the current file name should be reusable");
        let gap = allocate_title_bound_file_name(
            directory.path(),
            std::path::Path::new(""),
            "项目",
            DocumentKind::Markdown,
            None,
            &catalog_paths,
        )
        .expect("directory should be readable")
        .expect("the stable numbering gap should be available");

        assert_eq!(current.disambiguator, 1);
        assert_eq!(gap.disambiguator, 2);
        assert!(directory.path().join("项目 (3).md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn allocator_treats_a_symbolic_link_as_occupied() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("allocation directory should be created");
        fs::write(directory.path().join("target.md"), "target")
            .expect("symlink target fixture should be written");
        symlink(directory.path().join("target.md"), directory.path().join("链接.md"))
            .expect("symlink occupancy fixture should be created");

        let allocated = allocate_title_bound_file_name(
            directory.path(),
            std::path::Path::new(""),
            "链接",
            DocumentKind::Markdown,
            None,
            &[],
        )
        .expect("directory should be readable")
        .expect("a numbered name should remain available");

        assert_eq!(allocated.file_name, "链接 (2).md");
    }
}
