use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::DocumentKind;

pub const MAX_EXCERPT_GRAPHEMES: usize = 160;

/// 由后台索引器预计算、供卡片显示的文本摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteTextSummary {
    pub title: String,
    pub excerpt: String,
}

/// 标题栏与正文共享的标题投影结果；范围始终是 UTF-8 字节范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentTitleProjection {
    pub title: String,
    pub title_content_range: Option<Range<usize>>,
    pub insertion_byte_offset: usize,
}

/// 从完整源码投影文档标题，不访问文件名或文件系统。
pub fn document_title_projection(kind: DocumentKind, source: &str) -> DocumentTitleProjection {
    let title_content_range = match kind {
        DocumentKind::Text => first_text_line_range(source),
        DocumentKind::Markdown | DocumentKind::Mindmap => first_heading_range(source),
    };
    let title = title_content_range
        .as_ref()
        .map(|range| source[range.clone()].to_owned())
        .unwrap_or_default();
    DocumentTitleProjection { title, title_content_range, insertion_byte_offset: 0 }
}

/// 生成标题提交后的源码；空标题在最终规范化时变为“无标题”。
pub fn replace_document_title(kind: DocumentKind, source: &str, title: &str) -> String {
    let normalized_title = normalized_title(title);
    let projection = document_title_projection(kind, source);
    if let Some(range) = projection.title_content_range {
        let mut replaced_source = source.to_owned();
        replaced_source.replace_range(range, &normalized_title);
        return replaced_source;
    }

    if kind == DocumentKind::Mindmap
        && let Some(range) = first_marker_only_root_range(source)
    {
        let mut replaced_source = source.to_owned();
        replaced_source.replace_range(range, &format!("# {normalized_title}"));
        return replaced_source;
    }

    match kind {
        DocumentKind::Text => format!("{normalized_title}\n{source}"),
        DocumentKind::Markdown | DocumentKind::Mindmap => {
            format!("# {normalized_title}\n\n{source}")
        }
    }
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

fn normalized_title(title: &str) -> String {
    let trimmed_title = title.trim();
    if trimmed_title.is_empty() {
        return "无标题".to_owned();
    }
    trimmed_title.to_owned()
}

fn first_text_line_range(source: &str) -> Option<Range<usize>> {
    let first_line = source.split_inclusive('\n').next()?;
    let content_end = line_content_end(first_line);
    let content = &first_line[..content_end];
    Some(trimmed_range(0, content))
}

fn first_heading_range(source: &str) -> Option<Range<usize>> {
    let mut line_start = 0;
    for line in source.split_inclusive('\n') {
        let content_end = line_content_end(line);
        let content = &line[..content_end];
        if let Some(range) = heading_content_range(line_start, content) {
            return Some(range);
        }
        line_start += line.len();
    }
    None
}

fn first_marker_only_root_range(source: &str) -> Option<Range<usize>> {
    let mut line_start = 0;
    for line in source.split_inclusive('\n') {
        let content_end = line_content_end(line);
        let content = &line[..content_end];
        if content.trim() == "#" {
            let indentation = content.len() - content.trim_start().len();
            return Some(line_start + indentation..line_start + content_end);
        }
        line_start += line.len();
    }
    None
}

fn heading_content_range(line_start: usize, content: &str) -> Option<Range<usize>> {
    let indentation = content.len() - content.trim_start().len();
    let heading = content[indentation..].strip_prefix("# ")?;
    let heading_start = line_start + indentation + 2;
    Some(trimmed_range(heading_start, heading))
}

fn trimmed_range(start: usize, value: &str) -> Range<usize> {
    let leading_trimmed_bytes = value.len() - value.trim_start().len();
    let trimmed_value = value.trim();
    let trimmed_start = start + leading_trimmed_bytes;
    trimmed_start..trimmed_start + trimmed_value.len()
}

fn line_content_end(line: &str) -> usize {
    let without_line_feed = line.strip_suffix('\n').unwrap_or(line);
    without_line_feed.strip_suffix('\r').map_or(without_line_feed.len(), str::len)
}

#[cfg(test)]
mod tests {
    use unicode_segmentation::UnicodeSegmentation;

    use super::{
        MAX_EXCERPT_GRAPHEMES, document_title_projection, parse_note_text_summary,
        replace_document_title,
    };
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

    #[test]
    fn title_projection_uses_utf8_ranges_for_markdown_and_inserts_a_canonical_heading() {
        let source = "正文\r\n# 后一个\r\n";
        let projection = document_title_projection(DocumentKind::Markdown, source);

        assert_eq!(projection.title, "后一个");
        assert_eq!(
            &source[projection.title_content_range.clone().expect("heading range should exist")],
            "后一个"
        );
        assert_eq!(projection.insertion_byte_offset, 0);
        assert_eq!(
            replace_document_title(DocumentKind::Markdown, source, "新标题"),
            "正文\r\n# 新标题\r\n"
        );
        assert_eq!(
            replace_document_title(DocumentKind::Markdown, "正文\n", ""),
            "# 无标题\n\n正文\n"
        );
    }

    #[test]
    fn title_projection_uses_the_first_heading_and_does_not_touch_later_headings() {
        let source = "# 第一\n\n# 第二\n";
        let projection = document_title_projection(DocumentKind::Mindmap, source);

        assert_eq!(projection.title, "第一");
        assert_eq!(
            replace_document_title(DocumentKind::Mindmap, source, "根节点"),
            "# 根节点\n\n# 第二\n"
        );
    }

    #[test]
    fn mindmap_title_replaces_the_marker_only_root_instead_of_adding_a_second_root() {
        assert_eq!(replace_document_title(DocumentKind::Mindmap, "#", "根节点"), "# 根节点");
    }

    #[test]
    fn text_title_projection_targets_the_first_line_and_handles_an_empty_document() {
        let source = "  第一行  \r\n第二行";
        let projection = document_title_projection(DocumentKind::Text, source);

        assert_eq!(projection.title, "第一行");
        assert_eq!(
            &source[projection.title_content_range.clone().expect("text range should exist")],
            "第一行"
        );
        assert_eq!(
            replace_document_title(DocumentKind::Text, source, "标题"),
            "  标题  \r\n第二行"
        );
        assert_eq!(replace_document_title(DocumentKind::Text, "", ""), "无标题\n");
    }
}
