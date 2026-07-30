//! Syntax highlighting powered by the LSH (Lightweight Syntax Highlighter) engine.
//!
//! This module provides:
//! - `Language` and `HighlightKind` types auto-generated from `.lsh` definition files
//! - A `Highlighter` that wraps the LSH bytecode runtime for line-by-line highlighting
//! - A `HighlighterCache` for incremental re-highlighting

pub mod cache;
pub mod definitions;
pub mod highlighter;

pub use cache::HighlighterCache;
pub use highlighter::Highlighter;
pub use lsh::runtime::Language;

pub use lsh::runtime::Highlight;

/// Returns the scope name string for a given HighlightKind, used for theme color lookup.
pub fn highlight_kind_scope(kind: HighlightKind) -> &'static str {
    match kind {
        HighlightKind::Comment => "comment",
        HighlightKind::String => "string",
        HighlightKind::KeywordControl => "keyword.control",
        HighlightKind::KeywordOther => "keyword.other",
        HighlightKind::ConstantNumeric => "constant.numeric",
        HighlightKind::ConstantLanguage => "constant.language",
        HighlightKind::Variable => "variable",
        HighlightKind::Method => "method",
        HighlightKind::MetaHeader => "meta.header",
        HighlightKind::MarkupHeading => "markup.heading",
        HighlightKind::MarkupBold => "markup.bold",
        HighlightKind::MarkupItalic => "markup.italic",
        HighlightKind::MarkupList => "markup.list",
        HighlightKind::MarkupLink => "markup.link",
        HighlightKind::MarkupStrikethrough => "markup.strikethrough",
        HighlightKind::MarkupChanged => "markup.changed",
        HighlightKind::MarkupDeleted => "markup.deleted",
        HighlightKind::MarkupInserted => "markup.inserted",
        HighlightKind::Property => "property",
        HighlightKind::VariableSpecial => "variable.special",
        HighlightKind::KeywordImport => "keyword.import",
        HighlightKind::KeywordDeclaration => "keyword.declaration",
        HighlightKind::Boolean => "boolean",
        _ => "other",
    }
}
pub use definitions::{FILE_ASSOCIATIONS, HighlightKind, LANGUAGES};

/// Find a `Language` by a markdown fenced-code-block tag.
///
/// 3-level fuzzy match:
/// 1. Exact `id` match (e.g., "python", "javascript")
/// 2. Case-insensitive `name` match (e.g., "JavaScript", "Shell Script")
/// 3. File-extension alias via `FILE_ASSOCIATIONS` (e.g., "js", "py", "sh")
///
/// Returns `None` if no language matches (caller should render as plain text).
pub fn find_language(tag: &str) -> Option<&'static Language> {
    let tag_trimmed = tag.trim();
    if tag_trimmed.is_empty() {
        return None;
    }

    // 1. Exact id match
    if let Some(lang) = LANGUAGES.iter().find(|l| l.id == tag_trimmed) {
        return Some(lang);
    }

    // 2. Case-insensitive name match
    let tag_lower = tag_trimmed.to_ascii_lowercase();
    if let Some(lang) = LANGUAGES.iter().find(|l| l.name.to_ascii_lowercase() == tag_lower) {
        return Some(lang);
    }

    // 3. Extension alias match — extract extension from FILE_ASSOCIATIONS glob patterns
    for (pattern, lang) in FILE_ASSOCIATIONS {
        // Pattern is like "**/*.js" — extract the extension part
        if let Some(dot_pos) = pattern.rfind('.') {
            let ext = &pattern[dot_pos + 1..];
            if ext.eq_ignore_ascii_case(tag_trimmed) {
                return Some(lang);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_language_exact_id() {
        let lang = find_language("python");
        assert!(lang.is_some());
        assert_eq!(lang.unwrap().id, "python");
    }

    #[test]
    fn find_language_case_insensitive_name() {
        let lang = find_language("JavaScript");
        assert!(lang.is_some());
        assert_eq!(lang.unwrap().id, "javascript");
    }

    #[test]
    fn find_language_extension() {
        let lang = find_language("js");
        assert!(lang.is_some());
        assert_eq!(lang.unwrap().id, "javascript");
    }

    #[test]
    fn find_language_unknown_returns_none() {
        assert!(find_language("brainfuck").is_none());
    }

    #[test]
    fn find_language_empty_returns_none() {
        assert!(find_language("").is_none());
    }

    #[test]
    fn find_language_whitespace_trimmed() {
        let lang = find_language("  python  ");
        assert!(lang.is_some());
        assert_eq!(lang.unwrap().id, "python");
    }

    #[test]
    fn find_language_py_extension() {
        let lang = find_language("py");
        assert!(lang.is_some());
        assert_eq!(lang.unwrap().id, "python");
    }

    #[test]
    fn find_language_sh_extension() {
        let lang = find_language("sh");
        assert!(lang.is_some());
        assert_eq!(lang.unwrap().id, "shellscript");
    }
}
