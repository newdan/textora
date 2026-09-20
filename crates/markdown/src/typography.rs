//! Markdown source ranges that may receive natural typography spacing.

use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd, ParsedMarkdown};
use std::collections::HashMap;
use std::ops::Range;

/// A UTF-8 byte range in the original Markdown source.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct SourceRange {
    pub start: usize,
    pub end: usize,
}

impl SourceRange {
    fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end);
        Self { start, end }
    }
}

/// Prose and protected ranges for one parsed Markdown source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProseRanges {
    source_len: usize,
    prose: Vec<SourceRange>,
    protected: Vec<SourceRange>,
    char_boundaries: Vec<usize>,
}

impl ProseRanges {
    /// Length of the source in UTF-8 bytes.
    pub fn source_len(&self) -> usize {
        self.source_len
    }

    /// Ranges whose source spelling is eligible for natural typography.
    pub fn prose_ranges(&self) -> &[SourceRange] {
        &self.prose
    }

    /// Complement of [`Self::prose_ranges`], including Markdown syntax.
    pub fn protected_ranges(&self) -> &[SourceRange] {
        &self.protected
    }

    /// Return prose ranges intersecting a global source span.
    pub fn prose_ranges_in(&self, source_span: Range<usize>) -> Vec<SourceRange> {
        self.intersections(&self.prose, source_span)
    }

    /// Return a line or worker slice using local byte offsets.
    ///
    /// The input uses global source offsets. The returned ranges are relative to
    /// the normalized span and always include the span's protected complement.
    pub fn local_ranges_in(&self, source_span: Range<usize>) -> LocalSourceRanges {
        let span = self.normalize_span(source_span);
        LocalSourceRanges {
            prose: self.local_intersections(&self.prose, &span),
            protected: self.local_intersections(&self.protected, &span),
            source_span: SourceRange::new(span.start, span.end),
        }
    }

    fn normalize_span(&self, source_span: Range<usize>) -> Range<usize> {
        let requested_start = source_span.start.min(self.source_len);
        let requested_end = source_span.end.min(self.source_len).max(requested_start);
        let start = floor_boundary(&self.char_boundaries, requested_start);
        let end = ceil_boundary(&self.char_boundaries, requested_end);
        start..end
    }

    fn intersections(&self, ranges: &[SourceRange], source_span: Range<usize>) -> Vec<SourceRange> {
        let span = self.normalize_span(source_span);
        intersect_ranges(ranges, &span, false)
    }

    fn local_intersections(&self, ranges: &[SourceRange], span: &Range<usize>) -> Vec<SourceRange> {
        intersect_ranges(ranges, span, true)
    }
}

/// Prose and protected ranges relative to one global source slice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalSourceRanges {
    source_span: SourceRange,
    prose: Vec<SourceRange>,
    protected: Vec<SourceRange>,
}

impl LocalSourceRanges {
    /// Global source span represented by these local ranges.
    pub fn source_span(&self) -> SourceRange {
        self.source_span
    }

    /// Prose ranges relative to `source_span.start`.
    pub fn prose_ranges(&self) -> &[SourceRange] {
        &self.prose
    }

    /// Protected ranges relative to `source_span.start`.
    pub fn protected_ranges(&self) -> &[SourceRange] {
        &self.protected
    }
}

/// Extract source ranges that represent visible Markdown prose.
pub fn extract_prose_ranges(parsed: &ParsedMarkdown) -> ProseRanges {
    let source_len = parsed.source.len();
    let char_boundaries = source_boundaries(&parsed.source);
    let html_ranges = html_ranges(parsed);
    let known_code_ranges = known_code_ranges(parsed);
    let suspicious_code_ranges = suspicious_code_ranges(&parsed.source, &known_code_ranges);
    let extra_protected =
        merge_ranges(html_ranges.into_iter().chain(suspicious_code_ranges).collect());
    let mut candidates = Vec::new();
    let mut tag_stack = Vec::new();
    let mut protected_cursor = 0;

    for (event, source_range) in parsed.events.iter().zip(&parsed.event_ranges) {
        let source_range = clamp_source_range(source_range.clone(), source_len);
        match event {
            MarkdownEvent::Start(tag) => tag_stack.push(tag.clone()),
            MarkdownEvent::End(end) => pop_tag(&mut tag_stack, end),
            MarkdownEvent::Text(text) => {
                if text_is_prose(
                    &parsed.source,
                    &source_range,
                    text,
                    &tag_stack,
                    &extra_protected,
                    &mut protected_cursor,
                ) {
                    candidates.push(SourceRange::new(source_range.start, source_range.end));
                }
            }
            MarkdownEvent::Code(_)
            | MarkdownEvent::InlineHtml(_)
            | MarkdownEvent::SoftBreak
            | MarkdownEvent::HardBreak
            | MarkdownEvent::Rule
            | MarkdownEvent::TaskListMarker(_) => {}
        }
    }

    let prose = subtract_ranges(merge_ranges(candidates), &extra_protected);
    let protected = complement_ranges(source_len, &prose);
    ProseRanges { source_len, prose, protected, char_boundaries }
}

fn known_code_ranges(parsed: &ParsedMarkdown) -> Vec<SourceRange> {
    let mut block_starts = Vec::new();
    let mut ranges = Vec::new();
    for (event, range) in parsed.events.iter().zip(&parsed.event_ranges) {
        let range = SourceRange::new(range.start, range.end);
        match event {
            MarkdownEvent::Code(_) => ranges.push(range),
            MarkdownEvent::Start(MarkdownTag::CodeBlock { .. }) => block_starts.push(range.start),
            MarkdownEvent::End(MarkdownTagEnd::CodeBlock) => {
                if let Some(start) = block_starts.pop() {
                    ranges.push(SourceRange::new(start, range.end));
                }
            }
            _ => {}
        }
    }
    ranges
        .extend(block_starts.into_iter().map(|start| SourceRange::new(start, parsed.source.len())));
    merge_ranges(ranges)
}

fn text_is_prose(
    source: &str,
    source_range: &Range<usize>,
    rendered_text: &str,
    tag_stack: &[MarkdownTag],
    extra_protected: &[SourceRange],
    protected_cursor: &mut usize,
) -> bool {
    if source_range.start >= source_range.end
        || source.get(source_range.clone()) != Some(rendered_text)
        || overlaps_any(extra_protected, source_range, protected_cursor)
    {
        return false;
    }
    if tag_stack.iter().any(|tag| {
        matches!(
            tag,
            MarkdownTag::CodeBlock { .. }
                | MarkdownTag::MetadataBlock(_)
                | MarkdownTag::Image { .. }
        )
    }) {
        return false;
    }
    let Some(MarkdownTag::Link { url, .. }) =
        tag_stack.iter().rev().find(|tag| matches!(tag, MarkdownTag::Link { .. }))
    else {
        return true;
    };
    // `<https://example.test>` is represented as a Link event whose visible
    // text equals its destination. A normal label may still contain a URL.
    url != rendered_text
}

fn pop_tag(stack: &mut Vec<MarkdownTag>, end: &MarkdownTagEnd) {
    let expected = match end {
        MarkdownTagEnd::Paragraph => |tag: &MarkdownTag| matches!(tag, MarkdownTag::Paragraph),
        MarkdownTagEnd::Heading => |tag: &MarkdownTag| matches!(tag, MarkdownTag::Heading { .. }),
        MarkdownTagEnd::BlockQuote => |tag: &MarkdownTag| matches!(tag, MarkdownTag::BlockQuote),
        MarkdownTagEnd::CodeBlock => {
            |tag: &MarkdownTag| matches!(tag, MarkdownTag::CodeBlock { .. })
        }
        MarkdownTagEnd::List => |tag: &MarkdownTag| matches!(tag, MarkdownTag::List(..)),
        MarkdownTagEnd::Item => |tag: &MarkdownTag| matches!(tag, MarkdownTag::Item),
        MarkdownTagEnd::Table => |tag: &MarkdownTag| matches!(tag, MarkdownTag::Table(..)),
        MarkdownTagEnd::TableHead => |tag: &MarkdownTag| matches!(tag, MarkdownTag::TableHead),
        MarkdownTagEnd::TableRow => |tag: &MarkdownTag| matches!(tag, MarkdownTag::TableRow),
        MarkdownTagEnd::TableCell => |tag: &MarkdownTag| matches!(tag, MarkdownTag::TableCell),
        MarkdownTagEnd::Emphasis => |tag: &MarkdownTag| matches!(tag, MarkdownTag::Emphasis),
        MarkdownTagEnd::Strong => |tag: &MarkdownTag| matches!(tag, MarkdownTag::Strong),
        MarkdownTagEnd::Strikethrough => {
            |tag: &MarkdownTag| matches!(tag, MarkdownTag::Strikethrough)
        }
        MarkdownTagEnd::MetadataBlock(_) => {
            |tag: &MarkdownTag| matches!(tag, MarkdownTag::MetadataBlock(_))
        }
        MarkdownTagEnd::Link => |tag: &MarkdownTag| matches!(tag, MarkdownTag::Link { .. }),
        MarkdownTagEnd::Image => |tag: &MarkdownTag| matches!(tag, MarkdownTag::Image { .. }),
    };
    if let Some(index) = stack.iter().rposition(expected) {
        stack.truncate(index);
    }
}

fn html_ranges(parsed: &ParsedMarkdown) -> Vec<SourceRange> {
    let mut open_tags: Vec<(String, usize)> = Vec::new();
    let mut protected = Vec::new();
    let mut block_cursor = 0;
    for (event, range) in parsed.events.iter().zip(&parsed.event_ranges) {
        if !matches!(event, MarkdownEvent::InlineHtml(_)) {
            continue;
        }
        let Some(raw) = parsed.source.get(range.clone()) else { continue };
        while block_cursor < parsed.html_block_ranges.len()
            && parsed.html_block_ranges[block_cursor].end <= range.start
        {
            block_cursor += 1;
        }
        if parsed
            .html_block_ranges
            .get(block_cursor)
            .is_some_and(|block| block.start == range.start && block.end == range.end)
        {
            protected.push(SourceRange::new(range.start, range.end));
            continue;
        }
        // InlineHtml may contain a complete HTML block, including both the
        // opening and closing tags. Protect the parser event itself and scan
        // every tag so a closed inline event cannot leave an open stack entry behind.
        protected.push(SourceRange::new(range.start, range.end));
        for (tag_offset, tag) in html_tag_fragments(raw) {
            let absolute_start = range.start + tag_offset;
            let trimmed = tag.trim();
            let Some(name) = html_tag_name(trimmed) else { continue };
            if trimmed.starts_with("</") {
                if let Some(index) = open_tags.iter().rposition(|(open_name, _)| open_name == &name)
                {
                    let (_, start) = open_tags.remove(index);
                    protected.push(SourceRange::new(start, range.end));
                }
                continue;
            }
            if !trimmed.ends_with("/>") && !is_void_html_tag(&name) {
                open_tags.push((name, absolute_start));
            }
        }
    }
    protected.extend(
        open_tags.into_iter().map(|(_, start)| SourceRange::new(start, parsed.source.len())),
    );
    protected
}

fn html_tag_fragments(raw: &str) -> Vec<(usize, &str)> {
    let mut fragments = Vec::new();
    let mut search_start = 0;
    while let Some(relative_start) = raw[search_start..].find('<') {
        let start = search_start + relative_start;
        let Some(relative_end) = raw[start..].find('>') else { break };
        let end = start + relative_end + 1;
        fragments.push((start, &raw[start..end]));
        search_start = end;
    }
    fragments
}

fn html_tag_name(raw: &str) -> Option<String> {
    let mut inner = raw.strip_prefix('<')?;
    if let Some(stripped) = inner.strip_prefix('/') {
        inner = stripped;
    }
    if inner.starts_with('!') || inner.starts_with('?') {
        return None;
    }
    let end = inner
        .find(|character: char| character.is_whitespace() || matches!(character, '>' | '/'))?;
    (end > 0).then(|| inner[..end].to_ascii_lowercase())
}

fn is_void_html_tag(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn suspicious_code_ranges(source: &str, known_code_ranges: &[SourceRange]) -> Vec<SourceRange> {
    let mut protected = Vec::new();
    let mut offset = 0;
    let mut known_code_cursor = 0;
    for line in source.split_inclusive('\n') {
        let line_end = offset + line.len();
        let mut runs = Vec::new();
        let bytes = line.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] != b'`' {
                index += 1;
                continue;
            }
            let start = index;
            let absolute_start = offset + start;
            if is_escaped_backtick(source, absolute_start) {
                protected.push(SourceRange::new(absolute_start, absolute_start + 1));
                index += 1;
                continue;
            }
            while index < bytes.len() && bytes[index] == b'`' {
                index += 1;
            }
            runs.push((index - start, start));
        }
        let mut usable_runs = Vec::new();
        for (run_index, &(run_length, start)) in runs.iter().enumerate() {
            let absolute_start = offset + start;
            while known_code_cursor < known_code_ranges.len()
                && known_code_ranges[known_code_cursor].end <= absolute_start
            {
                known_code_cursor += 1;
            }
            let is_known_code = known_code_ranges
                .get(known_code_cursor)
                .is_some_and(|range| range.start <= absolute_start);
            if !is_known_code {
                usable_runs.push((run_index, run_length, start));
            }
        }
        let mut last_run_by_length = HashMap::new();
        for &(run_index, run_length, _) in &usable_runs {
            last_run_by_length.insert(run_length, run_index);
        }
        for &(run_index, run_length, _) in &usable_runs {
            if last_run_by_length.get(&run_length) == Some(&run_index) {
                let end = if run_length >= 3 { source.len() } else { line_end };
                protected.push(SourceRange::new(offset, end));
            }
        }
        offset = line_end;
    }
    protected
}

fn is_escaped_backtick(source: &str, backtick_offset: usize) -> bool {
    let mut slash_count = 0;
    for character in source[..backtick_offset].chars().rev() {
        if character != '\\' {
            break;
        }
        slash_count += 1;
    }
    slash_count % 2 == 1
}

fn source_boundaries(source: &str) -> Vec<usize> {
    source.char_indices().map(|(index, _)| index).chain([source.len()]).collect()
}

fn floor_boundary(boundaries: &[usize], offset: usize) -> usize {
    let index = boundaries.partition_point(|boundary| *boundary <= offset).saturating_sub(1);
    boundaries[index]
}

fn ceil_boundary(boundaries: &[usize], offset: usize) -> usize {
    let index = boundaries.partition_point(|boundary| *boundary < offset).min(boundaries.len() - 1);
    boundaries[index]
}

fn clamp_source_range(range: Range<usize>, source_len: usize) -> Range<usize> {
    let start = range.start.min(source_len);
    let end = range.end.min(source_len).max(start);
    start..end
}

fn overlaps_any(ranges: &[SourceRange], target: &Range<usize>, cursor: &mut usize) -> bool {
    while *cursor < ranges.len() && ranges[*cursor].end <= target.start {
        *cursor += 1;
    }
    ranges.get(*cursor).is_some_and(|range| range.start < target.end)
}

fn intersect_ranges(ranges: &[SourceRange], span: &Range<usize>, local: bool) -> Vec<SourceRange> {
    let first = ranges.partition_point(|range| range.end <= span.start);
    ranges[first..]
        .iter()
        .take_while(|range| range.start < span.end)
        .filter_map(|range| {
            let start = range.start.max(span.start);
            let end = range.end.min(span.end);
            (start < end).then(|| {
                if local {
                    SourceRange::new(start - span.start, end - span.start)
                } else {
                    SourceRange::new(start, end)
                }
            })
        })
        .collect()
}

fn merge_ranges(mut ranges: Vec<SourceRange>) -> Vec<SourceRange> {
    ranges.retain(|range| range.start < range.end);
    if !ranges.windows(2).all(|pair| pair[0] <= pair[1]) {
        ranges.sort_unstable();
    }
    let mut merged: Vec<SourceRange> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(previous) if range.start <= previous.end => {
                previous.end = previous.end.max(range.end)
            }
            _ => merged.push(range),
        }
    }
    merged
}

fn subtract_ranges(ranges: Vec<SourceRange>, protected: &[SourceRange]) -> Vec<SourceRange> {
    let mut result = Vec::new();
    let mut protected_cursor = 0;
    for range in ranges {
        let mut cursor = range.start;
        while protected_cursor < protected.len() && protected[protected_cursor].end <= cursor {
            protected_cursor += 1;
        }
        let mut blocked_index = protected_cursor;
        while let Some(blocked) = protected.get(blocked_index) {
            if blocked.start >= range.end {
                break;
            }
            if blocked.start > cursor {
                result.push(SourceRange::new(cursor, blocked.start.min(range.end)));
            }
            cursor = cursor.max(blocked.end);
            if cursor >= range.end {
                break;
            }
            blocked_index += 1;
        }
        if cursor < range.end {
            result.push(SourceRange::new(cursor, range.end));
        }
        protected_cursor = blocked_index;
    }
    result
}

fn complement_ranges(source_len: usize, ranges: &[SourceRange]) -> Vec<SourceRange> {
    let mut result = Vec::new();
    let mut cursor = 0;
    for range in ranges {
        if cursor < range.start {
            result.push(SourceRange::new(cursor, range.start));
        }
        cursor = cursor.max(range.end);
    }
    if cursor < source_len {
        result.push(SourceRange::new(cursor, source_len));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_markdown;

    fn ranges(source: &str) -> ProseRanges {
        extract_prose_ranges(&parse_markdown(source))
    }

    fn source_slices<'source>(source: &'source str, ranges: &[SourceRange]) -> Vec<&'source str> {
        ranges.iter().map(|range| &source[range.start..range.end]).collect()
    }

    #[test]
    fn extracts_plain_heading_and_list_text_but_not_markers() {
        let source = "# 标题\n- 项目\n> 引用";
        let extracted = ranges(source);

        assert_eq!(source_slices(source, extracted.prose_ranges()), ["标题", "项目", "引用"]);
        assert_eq!(source_slices(source, extracted.protected_ranges()), ["# ", "\n- ", "\n> "]);
    }

    #[test]
    fn extracts_emphasis_and_link_labels_but_protects_markup_and_destinations() {
        let source = "**你好,世界** [链接,文字](https://例子.test/a)";
        let extracted = ranges(source);

        assert_eq!(
            source_slices(source, extracted.prose_ranges()),
            ["你好,世界", " ", "链接,文字"]
        );
        assert_eq!(
            source_slices(source, extracted.protected_ranges()),
            ["**", "**", "[", "](https://例子.test/a)"]
        );
    }

    #[test]
    fn protects_code_html_metadata_and_unclosed_code() {
        let source = "---\ntitle: 中文\n---\n正文\n\n`代码,中文`\n\n<div>HTML,中文</div>\n\n未闭合 `代码,中文";
        let extracted = ranges(source);

        assert_eq!(source_slices(source, extracted.prose_ranges()), ["正文"]);
        assert!(
            extracted
                .protected_ranges()
                .iter()
                .any(|range| source[range.start..range.end].contains("代码,中文"))
        );
        assert!(
            extracted
                .protected_ranges()
                .iter()
                .any(|range| source[range.start..range.end].contains("HTML,中文"))
        );
    }

    #[test]
    fn local_source_span_clips_to_prose_ranges() {
        let source = "前文 **强调,内容** 后文";
        let extracted = ranges(source);
        let start = source.find("强调").expect("test source has emphasis");
        let end = source.find("后文").expect("test source has trailing text");

        assert_eq!(
            source_slices(source, &extracted.prose_ranges_in(start..end)),
            ["强调,内容", " "]
        );
    }

    #[test]
    fn closed_code_does_not_protect_following_cjk_punctuation() {
        let source = "`代码,中文` 后文,继续";
        let extracted = ranges(source);

        assert_eq!(source_slices(source, extracted.prose_ranges()), [" 后文,继续"]);
        assert_eq!(source_slices(source, extracted.protected_ranges()), ["`代码,中文`"]);
    }

    #[test]
    fn void_html_does_not_protect_following_prose() {
        let source = "前文<br>后文,继续<img alt=\"图\">尾文";
        let extracted = ranges(source);

        assert_eq!(source_slices(source, extracted.prose_ranges()), ["前文", "后文,继续", "尾文"]);
    }

    #[test]
    fn closed_html_block_restores_following_prose() {
        let source = "<div>HTML,中文</div>\n\n后文,继续";
        let extracted = ranges(source);

        assert_eq!(source_slices(source, extracted.prose_ranges()), ["后文,继续"]);
    }

    #[test]
    fn html_tag_matching_is_case_insensitive() {
        let source = "前文<span>HTML,中文</SPAN>后文,继续";
        let extracted = ranges(source);

        assert_eq!(source_slices(source, extracted.prose_ranges()), ["前文", "后文,继续"]);
    }

    #[test]
    fn html_block_raw_text_does_not_create_nested_protection() {
        let source = "<script>const template = \"<span>\";</script>\n\n后文,继续";
        let extracted = ranges(source);

        assert_eq!(source_slices(source, extracted.prose_ranges()), ["后文,继续"]);
    }

    #[test]
    fn escaped_backtick_does_not_protect_following_prose() {
        let source = "前文\\`后文,继续";
        let extracted = ranges(source);

        assert_eq!(source_slices(source, extracted.prose_ranges()), ["前文", "后文,继续"]);
    }

    #[test]
    fn non_ascii_local_slice_uses_utf8_boundaries_and_protected_complement() {
        let source = "中文 **强调,内容** 末尾";
        let extracted = ranges(source);
        let emphasis_start = source.find("强调").expect("test source has emphasis");
        let emphasis_end = source.find("末尾").expect("test source has trailing prose");
        let local = extracted.local_ranges_in(emphasis_start - 1..emphasis_end - 1);

        assert_eq!(local.source_span(), SourceRange::new(emphasis_start - 1, emphasis_end - 1));
        assert_eq!(local.prose_ranges(), [SourceRange::new(1, 14)]);
        assert_eq!(local.protected_ranges(), [SourceRange::new(0, 1), SourceRange::new(14, 16)]);
    }
}
