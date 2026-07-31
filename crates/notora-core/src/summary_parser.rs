use unicode_segmentation::UnicodeSegmentation;

use crate::DocumentKind;

pub const MAX_EXCERPT_GRAPHEMES: usize = 160;

/// 由后台索引器预计算、供卡片显示的文本摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteTextSummary {
    pub title: String,
    pub excerpt: String,
}

/// 从已读取的笔记文本生成标题和简介。
///
/// 该函数不访问文件系统；扫描和索引任务负责读取内容并传入文件 stem。
pub fn parse_note_text_summary(
    kind: DocumentKind,
    file_stem: &str,
    contents: &str,
) -> NoteTextSummary {
    let lines: Vec<&str> = contents.lines().collect();
    let title_line_index = match kind {
        DocumentKind::Text => first_nonempty_line_index(&lines),
        DocumentKind::Markdown | DocumentKind::Mindmap => first_level_one_heading_index(&lines),
    };
    let title = title_line_index
        .and_then(|index| title_from_line(kind, lines[index]))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| file_stem.to_owned());
    let excerpt = first_excerpt(&lines, title_line_index, kind);

    NoteTextSummary { title, excerpt }
}

fn first_nonempty_line_index(lines: &[&str]) -> Option<usize> {
    lines.iter().position(|line| !line.trim().is_empty())
}

fn first_level_one_heading_index(lines: &[&str]) -> Option<usize> {
    lines.iter().position(|line| level_one_heading(line).is_some())
}

fn title_from_line(kind: DocumentKind, line: &str) -> Option<String> {
    match kind {
        DocumentKind::Text => Some(line.trim().to_owned()),
        DocumentKind::Markdown | DocumentKind::Mindmap => {
            level_one_heading(line).map(str::to_owned)
        }
    }
}

fn first_excerpt(lines: &[&str], title_line_index: Option<usize>, kind: DocumentKind) -> String {
    lines
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != title_line_index)
        .map(|(_, line)| match kind {
            DocumentKind::Text => line.trim().to_owned(),
            DocumentKind::Markdown | DocumentKind::Mindmap => normalize_markdown_excerpt(line),
        })
        .find(|line| !line.is_empty())
        .map(|line| truncate_graphemes(&line, MAX_EXCERPT_GRAPHEMES))
        .unwrap_or_default()
}

fn level_one_heading(line: &str) -> Option<&str> {
    let heading = line.trim().strip_prefix("# ")?.trim();
    (!heading.is_empty()).then_some(heading)
}

fn normalize_markdown_excerpt(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() || is_markdown_heading(trimmed) {
        return String::new();
    }

    let without_quote = trimmed.strip_prefix(">").map_or(trimmed, str::trim_start);
    let without_list_marker = without_quote
        .strip_prefix("- ")
        .or_else(|| without_quote.strip_prefix("* "))
        .map_or(without_quote, str::trim_start);
    without_list_marker
        .trim_matches(|character| matches!(character, '`' | '*' | '_'))
        .trim()
        .to_owned()
}

fn is_markdown_heading(line: &str) -> bool {
    let marker_count = line.chars().take_while(|character| *character == '#').count();
    marker_count > 0 && line.chars().nth(marker_count) == Some(' ')
}

fn truncate_graphemes(value: &str, maximum_graphemes: usize) -> String {
    value.graphemes(true).take(maximum_graphemes).collect()
}

#[cfg(test)]
mod tests {
    use unicode_segmentation::UnicodeSegmentation;

    use super::{MAX_EXCERPT_GRAPHEMES, parse_note_text_summary};
    use crate::DocumentKind;

    #[test]
    fn markdown_uses_first_level_heading_and_next_content_paragraph() {
        let summary = parse_note_text_summary(
            DocumentKind::Markdown,
            "fallback",
            "# 设计\n\n> *首个段落*\n\n## 忽略二级标题",
        );

        assert_eq!(summary.title, "设计");
        assert_eq!(summary.excerpt, "首个段落");
    }

    #[test]
    fn text_uses_first_nonempty_line_as_title() {
        let summary =
            parse_note_text_summary(DocumentKind::Text, "fallback", "\n\nFirst line\nSecond line");

        assert_eq!(summary.title, "First line");
        assert_eq!(summary.excerpt, "Second line");
    }

    #[test]
    fn empty_documents_fall_back_to_file_stem() {
        let summary = parse_note_text_summary(DocumentKind::Mindmap, "architecture", "\n\n");

        assert_eq!(summary.title, "architecture");
        assert!(summary.excerpt.is_empty());
    }

    #[test]
    fn excerpt_truncation_preserves_grapheme_boundaries() {
        let long_paragraph = "👩🏽‍💻".repeat(MAX_EXCERPT_GRAPHEMES + 1);
        let summary = parse_note_text_summary(
            DocumentKind::Markdown,
            "fallback",
            &format!("# Title\n\n{long_paragraph}"),
        );

        assert_eq!(summary.excerpt.graphemes(true).count(), MAX_EXCERPT_GRAPHEMES);
        assert_eq!(summary.excerpt, "👩🏽‍💻".repeat(MAX_EXCERPT_GRAPHEMES));
    }
}
