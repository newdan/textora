use std::ops::Range;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use unicode_segmentation::UnicodeSegmentation;

use crate::DocumentKind;

pub const MAX_EXCERPT_GRAPHEMES: usize = 160;

const DIRECT_HEADING_SEPARATOR: &str = " · ";

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
    let excerpt = match kind {
        DocumentKind::Text => text_excerpt(&lines, title_line_index),
        DocumentKind::Markdown => markdown_excerpt(contents),
        DocumentKind::Mindmap => mindmap_excerpt(contents),
    };

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

fn text_excerpt(lines: &[&str], title_line_index: Option<usize>) -> String {
    let Some(first_body_line_index) = title_line_index.map(|index| index + 1) else {
        return String::new();
    };
    let paragraph = lines
        .iter()
        .skip(first_body_line_index)
        .skip_while(|line| line.trim().is_empty())
        .take_while(|line| !line.trim().is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    finalize_excerpt(&paragraph)
}

fn level_one_heading(line: &str) -> Option<&str> {
    let heading = line.trim().strip_prefix("# ")?.trim();
    (!heading.is_empty()).then_some(heading)
}

fn markdown_excerpt(contents: &str) -> String {
    let projection = project_markdown_excerpt(contents);
    let selected_excerpt = projection
        .first_visible_block
        .unwrap_or_else(|| projection.direct_headings.join(DIRECT_HEADING_SEPARATOR));
    finalize_excerpt(&selected_excerpt)
}

fn mindmap_excerpt(contents: &str) -> String {
    let projection = project_markdown_excerpt(contents);
    let selected_excerpt = projection
        .root_visible_block
        .unwrap_or_else(|| projection.direct_headings.join(DIRECT_HEADING_SEPARATOR));
    finalize_excerpt(&selected_excerpt)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct MarkdownExcerptProjection {
    first_visible_block: Option<String>,
    root_visible_block: Option<String>,
    direct_headings: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct VisibleBlockBuffer {
    source: String,
    belongs_to_mindmap_root: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct MarkdownExcerptParserState {
    ignored_block_depth: usize,
    image_depth: usize,
    current_heading: Option<HeadingLevel>,
    heading_source: String,
    visible_block: Option<VisibleBlockBuffer>,
    mindmap_root_section_open: bool,
}

fn project_markdown_excerpt(contents: &str) -> MarkdownExcerptProjection {
    let mut projection = MarkdownExcerptProjection::default();
    let mut parser_state = MarkdownExcerptParserState::default();

    for event in Parser::new_ext(contents, excerpt_markdown_options()) {
        match event {
            Event::Start(tag) => handle_markdown_tag_start(tag, &mut parser_state, &mut projection),
            Event::End(tag_end) => {
                handle_markdown_tag_end(tag_end, &mut parser_state, &mut projection)
            }
            Event::Text(text) | Event::Code(text) => {
                append_visible_text(&text, &mut parser_state);
            }
            Event::SoftBreak | Event::HardBreak => append_visible_text(" ", &mut parser_state),
            Event::Html(_)
            | Event::InlineHtml(_)
            | Event::FootnoteReference(_)
            | Event::TaskListMarker(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::Rule => {}
        }
    }
    complete_visible_block(&mut parser_state, &mut projection);
    projection
}

fn excerpt_markdown_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
}

fn handle_markdown_tag_start(
    tag: Tag<'_>,
    parser_state: &mut MarkdownExcerptParserState,
    projection: &mut MarkdownExcerptProjection,
) {
    match tag {
        Tag::CodeBlock(_) | Tag::MetadataBlock(_) | Tag::Table(_) | Tag::FootnoteDefinition(_) => {
            parser_state.ignored_block_depth += 1;
        }
        Tag::Image { .. } => parser_state.image_depth += 1,
        Tag::Heading { level, .. } => {
            complete_visible_block(parser_state, projection);
            if level != HeadingLevel::H1 {
                parser_state.mindmap_root_section_open = false;
            }
            parser_state.current_heading = Some(level);
            parser_state.heading_source.clear();
        }
        Tag::Paragraph | Tag::Item => start_visible_block(parser_state),
        Tag::BlockQuote(_)
        | Tag::List(_)
        | Tag::TableHead
        | Tag::TableRow
        | Tag::TableCell
        | Tag::Emphasis
        | Tag::Strong
        | Tag::Strikethrough
        | Tag::Link { .. }
        | Tag::HtmlBlock
        | Tag::DefinitionList
        | Tag::DefinitionListTitle
        | Tag::DefinitionListDefinition
        | Tag::Superscript
        | Tag::Subscript => {}
    }
}

fn handle_markdown_tag_end(
    tag_end: TagEnd,
    parser_state: &mut MarkdownExcerptParserState,
    projection: &mut MarkdownExcerptProjection,
) {
    match tag_end {
        TagEnd::CodeBlock
        | TagEnd::MetadataBlock(_)
        | TagEnd::Table
        | TagEnd::FootnoteDefinition => {
            parser_state.ignored_block_depth = parser_state.ignored_block_depth.saturating_sub(1);
        }
        TagEnd::Image => parser_state.image_depth = parser_state.image_depth.saturating_sub(1),
        TagEnd::Heading(level) => complete_heading(level, parser_state, projection),
        TagEnd::Paragraph | TagEnd::Item => complete_visible_block(parser_state, projection),
        TagEnd::BlockQuote(_)
        | TagEnd::List(_)
        | TagEnd::TableHead
        | TagEnd::TableRow
        | TagEnd::TableCell
        | TagEnd::Emphasis
        | TagEnd::Strong
        | TagEnd::Strikethrough
        | TagEnd::Link
        | TagEnd::HtmlBlock
        | TagEnd::DefinitionList
        | TagEnd::DefinitionListTitle
        | TagEnd::DefinitionListDefinition
        | TagEnd::Superscript
        | TagEnd::Subscript => {}
    }
}

fn start_visible_block(parser_state: &mut MarkdownExcerptParserState) {
    if parser_state.ignored_block_depth > 0
        || parser_state.current_heading.is_some()
        || parser_state.visible_block.is_some()
    {
        return;
    }
    parser_state.visible_block = Some(VisibleBlockBuffer {
        source: String::new(),
        belongs_to_mindmap_root: parser_state.mindmap_root_section_open,
    });
}

fn append_visible_text(fragment: &str, parser_state: &mut MarkdownExcerptParserState) {
    if parser_state.ignored_block_depth > 0 || parser_state.image_depth > 0 {
        return;
    }
    if parser_state.current_heading.is_some() {
        parser_state.heading_source.push_str(fragment);
        return;
    }
    if let Some(visible_block) = &mut parser_state.visible_block {
        visible_block.source.push_str(fragment);
    }
}

fn complete_heading(
    level: HeadingLevel,
    parser_state: &mut MarkdownExcerptParserState,
    projection: &mut MarkdownExcerptProjection,
) {
    let heading = normalize_visible_text(&parser_state.heading_source);
    if level == HeadingLevel::H2 && !heading.is_empty() {
        projection.direct_headings.push(heading);
    }
    if level == HeadingLevel::H1 {
        parser_state.mindmap_root_section_open = true;
    }
    parser_state.current_heading = None;
    parser_state.heading_source.clear();
}

fn complete_visible_block(
    parser_state: &mut MarkdownExcerptParserState,
    projection: &mut MarkdownExcerptProjection,
) {
    let Some(visible_block) = parser_state.visible_block.take() else {
        return;
    };
    let visible_text = normalize_visible_text(&visible_block.source);
    if visible_text.is_empty() {
        return;
    }
    if projection.first_visible_block.is_none() {
        projection.first_visible_block = Some(visible_text.clone());
    }
    if visible_block.belongs_to_mindmap_root && projection.root_visible_block.is_none() {
        projection.root_visible_block = Some(visible_text);
    }
}

fn finalize_excerpt(source: &str) -> String {
    let normalized = normalize_visible_text(source);
    truncate_graphemes(&normalized, MAX_EXCERPT_GRAPHEMES)
}

fn normalize_visible_text(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
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
    fn markdown_extracts_visible_prose_instead_of_source_markers() {
        let summary = parse_note_text_summary(
            DocumentKind::Markdown,
            "fallback",
            r#"---
tags: [design]
---
# 设计文档

![架构图](architecture.png)

```rust
let source_marker = "must not leak";
```

第一段包含 **重点**、[链接](https://example.com) 和 `inline code`。
"#,
        );

        assert_eq!(summary.excerpt, "第一段包含 重点、链接 和 inline code。");
    }

    #[test]
    fn markdown_task_list_uses_task_text_without_checkbox_marker() {
        let summary = parse_note_text_summary(
            DocumentKind::Markdown,
            "fallback",
            "# Tasks\n\n- [x] 完成索引重构\n- [ ] 补充性能测试\n",
        );

        assert_eq!(summary.excerpt, "完成索引重构");
    }

    #[test]
    fn markdown_outline_falls_back_to_direct_section_titles() {
        let summary = parse_note_text_summary(
            DocumentKind::Markdown,
            "fallback",
            "# 产品设计\n\n## 需求分析\n\n### 边界情况\n\n## 发布计划\n",
        );

        assert_eq!(summary.excerpt, "需求分析 · 发布计划");
    }

    #[test]
    fn text_uses_first_nonempty_line_as_title() {
        let summary =
            parse_note_text_summary(DocumentKind::Text, "fallback", "\n\nFirst line\nSecond line");

        assert_eq!(summary.title, "First line");
        assert_eq!(summary.excerpt, "Second line");
    }

    #[test]
    fn text_excerpt_joins_the_first_body_paragraph() {
        let summary = parse_note_text_summary(
            DocumentKind::Text,
            "fallback",
            "\n周会记录\n\n今天讨论了搜索体验，\n以及索引性能的后续安排。\n\n下一段不应进入摘要。",
        );

        assert_eq!(summary.title, "周会记录");
        assert_eq!(summary.excerpt, "今天讨论了搜索体验， 以及索引性能的后续安排。");
    }

    #[test]
    fn mindmap_prefers_root_note_over_branch_titles() {
        let summary = parse_note_text_summary(
            DocumentKind::Mindmap,
            "fallback",
            r#"```toml mindmap
theme = "dawn"
```

# 产品规划

这是根节点的 **整体说明**。

## 需求分析

子节点备注不应抢占根备注。

## 发布计划
"#,
        );

        assert_eq!(summary.title, "产品规划");
        assert_eq!(summary.excerpt, "这是根节点的 整体说明。");
    }

    #[test]
    fn mindmap_without_root_note_joins_direct_branch_titles() {
        let summary = parse_note_text_summary(
            DocumentKind::Mindmap,
            "fallback",
            r#"```toml mindmap
theme = "dawn"
```

# 产品规划

## 需求分析

```toml node
priority = "P1"
```

子节点备注不是根备注。

### 边界情况

## 发布计划
"#,
        );

        assert_eq!(summary.excerpt, "需求分析 · 发布计划");
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
