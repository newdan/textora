//! Markdown parser — wraps pulldown_cmark, producing an owned event list.

use pulldown_cmark::{Event, HeadingLevel, MetadataBlockKind, Options, Parser, Tag, TagEnd};
use std::ops::Range;

/// Owned version of pulldown_cmark events, free of lifetime constraints.
#[derive(Clone, Debug, PartialEq)]
pub enum MarkdownEvent {
    Start(MarkdownTag),
    End(MarkdownTagEnd),
    Text(String),
    Code(String),
    InlineHtml(String),
    SoftBreak { next_line_has_explicit_blockquote_marker: bool },
    HardBreak,
    Rule,
    TaskListMarker(bool),
}

/// Simplified owned tag (opening).
#[derive(Clone, Debug, PartialEq)]
pub enum MarkdownTag {
    Paragraph,
    Heading { level: u32 },
    BlockQuote,
    CodeBlock { info: Option<String> },
    List(Option<u64>, bool, bool), // (start, tight, blank_line_before)
    Item,
    Table(Vec<pulldown_cmark::Alignment>),
    TableHead,
    TableRow,
    TableCell,
    Emphasis,
    Strong,
    Strikethrough,
    MetadataBlock(MetadataBlockKind),
    Link { url: String, title: String },
    Image { url: String, title: String },
}

/// Simplified owned tag (closing).
#[derive(Clone, Debug, PartialEq)]
pub enum MarkdownTagEnd {
    Paragraph,
    Heading,
    BlockQuote,
    CodeBlock,
    List,
    Item,
    Table,
    TableHead,
    TableRow,
    TableCell,
    Emphasis,
    Strong,
    Strikethrough,
    MetadataBlock(MetadataBlockKind),
    Link,
    Image,
}

/// Result of parsing markdown text.
#[derive(Clone, Debug, Default)]
pub struct ParsedMarkdown {
    pub events: Vec<MarkdownEvent>,
    /// event_ranges[i] = events[i] 在源码中的完整字节区间。
    pub event_ranges: Vec<Range<usize>>,
}

/// Parse markdown text into an owned event list.
pub fn parse_markdown(src: &str) -> ParsedMarkdown {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    opts.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);

    let parser = Parser::new_ext(src, opts);
    let mut events = Vec::new();
    let mut event_ranges = Vec::new();

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if let Some(t) = convert_tag(tag) {
                    event_ranges.push(range.clone());
                    events.push(MarkdownEvent::Start(t));
                }
            }
            Event::End(tag_end) => {
                if let Some(t) = convert_tag_end(tag_end) {
                    event_ranges.push(range.clone());
                    events.push(MarkdownEvent::End(t));
                }
            }
            Event::Text(text) => {
                event_ranges.push(range.clone());
                events.push(MarkdownEvent::Text(text.into_string()));
            }
            Event::Code(code) => {
                event_ranges.push(range.clone());
                events.push(MarkdownEvent::Code(code.into_string()));
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                event_ranges.push(range.clone());
                events.push(MarkdownEvent::InlineHtml(html.into_string()));
            }
            Event::SoftBreak => {
                event_ranges.push(range.clone());
                events.push(MarkdownEvent::SoftBreak {
                    next_line_has_explicit_blockquote_marker:
                        next_line_has_explicit_blockquote_marker(src, &range),
                });
            }
            Event::HardBreak => {
                event_ranges.push(range.clone());
                events.push(MarkdownEvent::HardBreak);
            }
            Event::Rule => {
                event_ranges.push(range.clone());
                events.push(MarkdownEvent::Rule);
            }
            Event::TaskListMarker(checked) => {
                event_ranges.push(range.clone());
                events.push(MarkdownEvent::TaskListMarker(checked));
            }
            // Skip footnote refs, math, etc. for now
            _ => {}
        }
    }

    // Post-pass: detect tight vs loose lists and blank lines before lists.
    detect_list_properties(&mut events, src, &event_ranges);

    ParsedMarkdown { events, event_ranges }
}

fn next_line_has_explicit_blockquote_marker(src: &str, softbreak_range: &Range<usize>) -> bool {
    let Some(newline_offset) = src[softbreak_range.start..].find('\n') else {
        return false;
    };
    let next_line_start = softbreak_range.start + newline_offset + 1;
    matches!(src[next_line_start..].trim_start_matches([' ', '\t']).chars().next(), Some('>'))
}

fn convert_tag(tag: Tag<'_>) -> Option<MarkdownTag> {
    match tag {
        Tag::Paragraph => Some(MarkdownTag::Paragraph),
        Tag::Heading { level, .. } => {
            Some(MarkdownTag::Heading { level: heading_level_u32(level) })
        }
        Tag::BlockQuote(_) => Some(MarkdownTag::BlockQuote),
        Tag::CodeBlock(kind) => {
            let info = match kind {
                pulldown_cmark::CodeBlockKind::Fenced(cow) if !cow.is_empty() => {
                    Some(cow.into_string())
                }
                pulldown_cmark::CodeBlockKind::Fenced(_)
                | pulldown_cmark::CodeBlockKind::Indented => None,
            };
            Some(MarkdownTag::CodeBlock { info })
        }
        Tag::List(start) => Some(MarkdownTag::List(start, true, false)), // defaults, post-pass will fix
        Tag::Item => Some(MarkdownTag::Item),
        Tag::Table(alignments) => Some(MarkdownTag::Table(alignments)),
        Tag::TableHead => Some(MarkdownTag::TableHead),
        Tag::TableRow => Some(MarkdownTag::TableRow),
        Tag::TableCell => Some(MarkdownTag::TableCell),
        Tag::Emphasis => Some(MarkdownTag::Emphasis),
        Tag::Strong => Some(MarkdownTag::Strong),
        Tag::Strikethrough => Some(MarkdownTag::Strikethrough),
        Tag::MetadataBlock(kind) => Some(MarkdownTag::MetadataBlock(kind)),
        Tag::Link { dest_url, title, .. } => {
            Some(MarkdownTag::Link { url: dest_url.into_string(), title: title.into_string() })
        }
        Tag::Image { dest_url, title, .. } => {
            Some(MarkdownTag::Image { url: dest_url.into_string(), title: title.into_string() })
        }
        // Skip HTML blocks, footnotes, definition lists, metadata
        _ => None,
    }
}

fn convert_tag_end(te: TagEnd) -> Option<MarkdownTagEnd> {
    match te {
        TagEnd::Paragraph => Some(MarkdownTagEnd::Paragraph),
        TagEnd::Heading(_) => Some(MarkdownTagEnd::Heading),
        TagEnd::BlockQuote(_) => Some(MarkdownTagEnd::BlockQuote),
        TagEnd::CodeBlock => Some(MarkdownTagEnd::CodeBlock),
        TagEnd::List(_) => Some(MarkdownTagEnd::List),
        TagEnd::Item => Some(MarkdownTagEnd::Item),
        TagEnd::Table => Some(MarkdownTagEnd::Table),
        TagEnd::TableHead => Some(MarkdownTagEnd::TableHead),
        TagEnd::TableRow => Some(MarkdownTagEnd::TableRow),
        TagEnd::TableCell => Some(MarkdownTagEnd::TableCell),
        TagEnd::Emphasis => Some(MarkdownTagEnd::Emphasis),
        TagEnd::Strong => Some(MarkdownTagEnd::Strong),
        TagEnd::Strikethrough => Some(MarkdownTagEnd::Strikethrough),
        TagEnd::MetadataBlock(kind) => Some(MarkdownTagEnd::MetadataBlock(kind)),
        TagEnd::Link => Some(MarkdownTagEnd::Link),
        TagEnd::Image => Some(MarkdownTagEnd::Image),
        _ => None,
    }
}

fn heading_level_u32(level: HeadingLevel) -> u32 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn detect_list_properties(events: &mut [MarkdownEvent], src: &str, ranges: &[Range<usize>]) {
    // First pass: find all List start positions and determine tightness + blank_line_before
    let mut list_updates: Vec<(usize, bool, bool)> = Vec::new(); // (event_idx, tight, blank_line_before)
    let mut i = 0;
    while i < events.len() {
        if matches!(&events[i], MarkdownEvent::Start(MarkdownTag::List(_, _, _))) {
            let list_start = i;
            // Scan forward through this list's items to detect tightness
            let mut depth = 1u32;
            let mut j = i + 1;
            let mut is_tight = true;
            while j < events.len() && depth > 0 {
                match &events[j] {
                    MarkdownEvent::Start(MarkdownTag::List(..)) => depth += 1,
                    MarkdownEvent::End(MarkdownTagEnd::List) => depth -= 1,
                    MarkdownEvent::Start(MarkdownTag::Item)
                        if depth == 1
                        // Check if the next event after Item start is Paragraph
                        && j + 1 < events.len()
                            && matches!(
                                &events[j + 1],
                                MarkdownEvent::Start(MarkdownTag::Paragraph)
                            ) =>
                    {
                        is_tight = false;
                    }
                    _ => {}
                }
                j += 1;
            }
            // Detect blank line before list by checking source text
            let blank_before = if list_start < ranges.len() {
                has_blank_line_before_offset(src, ranges[list_start].start)
            } else {
                false
            };
            list_updates.push((list_start, is_tight, blank_before));
            i = j; // skip past End(List)
        } else {
            i += 1;
        }
    }
    // Second pass: apply updates
    for (idx, is_tight, blank_before) in list_updates {
        if let MarkdownEvent::Start(MarkdownTag::List(_, ref mut tight, ref mut blank)) =
            events[idx]
        {
            *tight = is_tight;
            *blank = blank_before;
        }
    }
}

/// Check if there is a blank line (empty line) in the source text before the given byte offset.
fn has_blank_line_before_offset(src: &str, offset: usize) -> bool {
    if offset == 0 {
        return false;
    }
    let before = &src[..offset];
    // Trim trailing whitespace, then check if the last two chars include a blank line
    let trimmed = before.trim_end();
    if trimmed.is_empty() {
        return false;
    }
    // Check if there's a blank line (\n\n) between trimmed content and the offset
    let gap = &src[trimmed.len()..offset];
    gap.contains("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_paragraph() {
        let parsed = parse_markdown("hello world");
        assert_eq!(parsed.events.len(), 3); // Start(Paragraph), Text, End(Paragraph)
        assert!(matches!(parsed.events[0], MarkdownEvent::Start(MarkdownTag::Paragraph)));
        assert!(matches!(parsed.events[1], MarkdownEvent::Text(ref t) if t == "hello world"));
        assert!(matches!(parsed.events[2], MarkdownEvent::End(MarkdownTagEnd::Paragraph)));
    }

    #[test]
    fn parse_softbreak_tracks_the_next_explicit_blockquote_marker() {
        let parsed = parse_markdown("> first\n> second");

        assert!(parsed.events.iter().any(|event| {
            matches!(
                event,
                MarkdownEvent::SoftBreak { next_line_has_explicit_blockquote_marker: true }
            )
        }));
    }

    #[test]
    fn parse_softbreak_keeps_a_lazy_blockquote_continuation_unmarked() {
        let parsed = parse_markdown("> first\nsecond");

        assert!(parsed.events.iter().any(|event| {
            matches!(
                event,
                MarkdownEvent::SoftBreak { next_line_has_explicit_blockquote_marker: false }
            )
        }));
    }

    #[test]
    fn parse_headings() {
        let parsed = parse_markdown("# H1\n## H2\n### H3");
        let heading_starts: Vec<_> = parsed
            .events
            .iter()
            .filter(|e| matches!(e, MarkdownEvent::Start(MarkdownTag::Heading { .. })))
            .collect();
        assert_eq!(heading_starts.len(), 3);
        assert!(matches!(
            heading_starts[0],
            MarkdownEvent::Start(MarkdownTag::Heading { level: 1 })
        ));
    }

    #[test]
    fn parse_bold_italic() {
        let parsed = parse_markdown("**bold** and *italic*");
        let has_strong =
            parsed.events.iter().any(|e| matches!(e, MarkdownEvent::Start(MarkdownTag::Strong)));
        let has_emphasis =
            parsed.events.iter().any(|e| matches!(e, MarkdownEvent::Start(MarkdownTag::Emphasis)));
        assert!(has_strong);
        assert!(has_emphasis);
    }

    #[test]
    fn parse_code_block() {
        let parsed = parse_markdown("```rust\nfn main() {}\n```");
        let has_code_block = parsed.events.iter().any(|e| {
            matches!(
                e,
                MarkdownEvent::Start(MarkdownTag::CodeBlock { info: Some(s) }) if s == "rust"
            )
        });
        assert!(has_code_block);
    }

    #[test]
    fn parse_indented_code_block_is_not_fenced() {
        let parsed = parse_markdown("    let value = 1;");
        assert!(parsed.events.iter().any(|event| {
            matches!(event, MarkdownEvent::Start(MarkdownTag::CodeBlock { info: None }))
        }));
    }

    #[test]
    fn code_block_public_variants_keep_their_original_field_shapes() {
        let _tag = MarkdownTag::CodeBlock { info: None };
        let _block = crate::builder::BlockKind::CodeBlock { language: None };
        let _laid_out =
            crate::layout::LaidOutBlockKind::CodeBlock { lines: Vec::new(), language: None };
    }

    #[test]
    fn parse_inline_code() {
        let parsed = parse_markdown("use `println!`");
        let has_code =
            parsed.events.iter().any(|e| matches!(e, MarkdownEvent::Code(s) if s == "println!"));
        assert!(has_code);
    }

    #[test]
    fn parse_table() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let parsed = parse_markdown(md);
        let has_table =
            parsed.events.iter().any(|e| matches!(e, MarkdownEvent::Start(MarkdownTag::Table(_))));
        assert!(has_table);
    }

    #[test]
    fn parse_unordered_list() {
        let parsed = parse_markdown("- item 1\n- item 2");
        let item_count = parsed
            .events
            .iter()
            .filter(|e| matches!(e, MarkdownEvent::Start(MarkdownTag::Item)))
            .count();
        assert_eq!(item_count, 2);
    }

    #[test]
    fn parse_rule() {
        // "---" after text is a thematic break (rule)
        let parsed = parse_markdown(
            "text

---",
        );
        let has_rule = parsed.events.iter().any(|e| matches!(e, MarkdownEvent::Rule));
        assert!(has_rule);
    }

    #[test]
    fn parse_yaml_metadata_block() {
        // "---" at document start is a YAML metadata block
        let parsed = parse_markdown(
            "---
title: hello
---",
        );
        let has_metadata = parsed.events.iter().any(|e| {
            matches!(
                e,
                MarkdownEvent::Start(MarkdownTag::MetadataBlock(MetadataBlockKind::YamlStyle))
            )
        });
        assert!(has_metadata, "YAML --- at document start should produce MetadataBlock");
        // Should NOT produce a Rule event
        let has_rule = parsed.events.iter().any(|e| matches!(e, MarkdownEvent::Rule));
        assert!(!has_rule, "YAML metadata block should not produce a Rule");
    }

    #[test]
    fn parse_link() {
        let parsed = parse_markdown("[example](https://example.com)");
        let has_link = parsed.events.iter().any(|e| {
            matches!(e, MarkdownEvent::Start(MarkdownTag::Link { url, .. }) if url == "https://example.com")
        });
        assert!(has_link);
    }
}
