use std::ops::Range;

use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd};
use ui::plugin::SemanticEditPlan;

#[derive(Clone, Copy)]
pub(super) enum InlineFormat {
    Bold,
    Italic,
    Strikethrough,
}

impl InlineFormat {
    fn marker(self) -> &'static str {
        match self {
            Self::Bold => "**",
            Self::Italic => "*",
            Self::Strikethrough => "~~",
        }
    }

    fn matches(self, event: &MarkdownEvent) -> bool {
        matches!(
            (self, event),
            (Self::Bold, MarkdownEvent::Start(MarkdownTag::Strong))
                | (Self::Italic, MarkdownEvent::Start(MarkdownTag::Emphasis))
                | (Self::Strikethrough, MarkdownEvent::Start(MarkdownTag::Strikethrough))
        )
    }

    fn span(self, source: &str, range: Range<usize>) -> FormatSpan {
        let marker_length = match self {
            Self::Bold | Self::Italic => self.marker().len(),
            Self::Strikethrough => {
                source[range.clone()].bytes().take_while(|byte| *byte == b'~').count()
            }
        };
        FormatSpan {
            opening: range.start..range.start + marker_length,
            closing: range.end - marker_length..range.end,
            range,
        }
    }
}

struct FormatSpan {
    range: Range<usize>,
    opening: Range<usize>,
    closing: Range<usize>,
}

struct InlineRanges {
    blocks: Vec<Range<usize>>,
    styles: Vec<FormatSpan>,
    visible_content: Vec<Range<usize>>,
}

enum ContentBlock {
    Inline(Option<Range<usize>>),
    Opaque,
}

pub(super) fn plan_toggle(
    source: &str,
    source_generation: u32,
    cursor_byte: usize,
    selection: Option<Range<usize>>,
    format: InlineFormat,
) -> SemanticEditPlan {
    let Some(selection) = selection.filter(|range| range.start < range.end) else {
        let marker = format.marker();
        return super::apply_transaction(
            source_generation,
            cursor_byte..cursor_byte,
            marker.repeat(2),
            cursor_byte + marker.len(),
        );
    };
    if source[selection.clone()] == *format.marker() {
        let replacement = format.marker().repeat(3);
        let cursor_after = selection.start + replacement.len();
        return super::apply_transaction(source_generation, selection, replacement, cursor_after);
    }
    let inline = inline_ranges(source, Some(format));
    let targets: Vec<_> = inline
        .blocks
        .iter()
        .filter_map(|block| selected_content(source, block.clone(), &selection, &inline.styles))
        .collect();
    let remove = targets.iter().all(|target| fully_formatted(source, target, &inline));
    plan_targets(source, source_generation, &targets, |target| {
        format_target(source, target, &inline.styles, format.marker(), remove)
    })
}

pub(super) fn selected_blocks(source: &str, selection: &Range<usize>) -> Vec<Range<usize>> {
    if !source[selection.clone()].contains('\n') {
        return vec![selection.clone()];
    }
    inline_ranges(source, None)
        .blocks
        .into_iter()
        .filter_map(|block| selected_content(source, block, selection, &[]))
        .collect()
}

pub(super) fn plan_block_transform(
    source: &str,
    source_generation: u32,
    targets: &[Range<usize>],
    transform: impl Fn(&str) -> String,
) -> SemanticEditPlan {
    plan_targets(source, source_generation, targets, |target| transform(&source[target.clone()]))
}

fn plan_targets(
    source: &str,
    source_generation: u32,
    targets: &[Range<usize>],
    transform: impl Fn(&Range<usize>) -> String,
) -> SemanticEditPlan {
    if targets.is_empty() {
        return SemanticEditPlan::NoChange;
    }
    let start = targets.first().expect("nonempty targets checked above").start;
    let end = targets.last().expect("nonempty targets checked above").end;
    let mut replacement = source[start..end].to_owned();
    for target in targets.iter().rev() {
        let formatted = transform(target);
        replacement.replace_range(target.start - start..target.end - start, &formatted);
    }
    if replacement == source[start..end] {
        return SemanticEditPlan::NoChange;
    }
    let cursor_after = start + replacement.len();
    super::apply_transaction(source_generation, start..end, replacement, cursor_after)
}

fn inline_ranges(source: &str, format: Option<InlineFormat>) -> InlineRanges {
    let parsed = crate::parser::parse_markdown(source);
    let mut inline =
        InlineRanges { blocks: Vec::new(), styles: Vec::new(), visible_content: Vec::new() };
    let mut containers = Vec::new();
    for (event, range) in parsed.events.iter().zip(&parsed.event_ranges) {
        match event {
            MarkdownEvent::Start(
                MarkdownTag::Paragraph
                | MarkdownTag::Heading { .. }
                | MarkdownTag::TableCell
                | MarkdownTag::Item,
            ) => {
                flush_inline_block(containers.last_mut(), &mut inline.blocks);
                containers.push(ContentBlock::Inline(None));
            }
            MarkdownEvent::Start(MarkdownTag::CodeBlock { .. }) => {
                flush_inline_block(containers.last_mut(), &mut inline.blocks);
                containers.push(ContentBlock::Opaque);
            }
            MarkdownEvent::End(
                MarkdownTagEnd::Paragraph
                | MarkdownTagEnd::Heading
                | MarkdownTagEnd::TableCell
                | MarkdownTagEnd::Item
                | MarkdownTagEnd::CodeBlock,
            ) => {
                flush_inline_block(containers.last_mut(), &mut inline.blocks);
                containers.pop();
            }
            _ => {
                let Some(ContentBlock::Inline(content)) = containers.last_mut() else {
                    continue;
                };
                record_inline_event(&mut inline, source, event, range, content, format);
            }
        }
    }
    inline
}

fn record_inline_event(
    inline: &mut InlineRanges,
    source: &str,
    event: &MarkdownEvent,
    range: &Range<usize>,
    content: &mut Option<Range<usize>>,
    format: Option<InlineFormat>,
) {
    if !is_inline_content(event) {
        return;
    }
    if let Some(format) = format.filter(|format| format.matches(event)) {
        inline.styles.push(format.span(source, range.clone()));
    }
    if matches!(
        event,
        MarkdownEvent::Text(_)
            | MarkdownEvent::Code(_)
            | MarkdownEvent::InlineMath(_)
            | MarkdownEvent::DisplayMath(_)
            | MarkdownEvent::InlineHtml(_)
    ) {
        inline.visible_content.push(range.clone());
    }
    match content {
        Some(content) => content.end = content.end.max(range.end),
        None => *content = Some(range.clone()),
    }
}

fn flush_inline_block(block: Option<&mut ContentBlock>, blocks: &mut Vec<Range<usize>>) {
    if let Some(ContentBlock::Inline(content)) = block
        && let Some(range) = content.take()
    {
        blocks.push(range);
    }
}

fn is_inline_content(event: &MarkdownEvent) -> bool {
    matches!(
        event,
        MarkdownEvent::Text(_)
            | MarkdownEvent::Code(_)
            | MarkdownEvent::InlineMath(_)
            | MarkdownEvent::DisplayMath(_)
            | MarkdownEvent::InlineHtml(_)
            | MarkdownEvent::Start(
                MarkdownTag::Emphasis
                    | MarkdownTag::Strong
                    | MarkdownTag::Strikethrough
                    | MarkdownTag::Link { .. }
                    | MarkdownTag::Image { .. }
            )
            | MarkdownEvent::End(
                MarkdownTagEnd::Emphasis
                    | MarkdownTagEnd::Strong
                    | MarkdownTagEnd::Strikethrough
                    | MarkdownTagEnd::Link
                    | MarkdownTagEnd::Image
            )
    )
}

fn fully_formatted(source: &str, target: &Range<usize>, inline: &InlineRanges) -> bool {
    inline.visible_content.iter().all(|content| {
        let selected = content.start.max(target.start)..content.end.min(target.end);
        selected.start >= selected.end
            || source[selected.clone()].trim().is_empty()
            || enclosing_style(&selected, &inline.styles).is_some()
    })
}

fn selected_content(
    source: &str,
    block: Range<usize>,
    selection: &Range<usize>,
    styles: &[FormatSpan],
) -> Option<Range<usize>> {
    let mut target = block.start.max(selection.start)..block.end.min(selection.end);
    if target.start >= target.end {
        return None;
    }
    let selected = &source[target.clone()];
    target.start += selected.len() - selected.trim_start().len();
    target.end -= selected.len() - selected.trim_end().len();
    if target.start >= target.end {
        return None;
    }
    for span in styles {
        if target.start >= span.opening.start && target.start <= span.opening.end {
            target.start = span.range.start;
        }
        if target.end >= span.closing.start && target.end <= span.closing.end {
            target.end = span.range.end;
        }
    }
    Some(target)
}

fn enclosing_style<'a>(target: &Range<usize>, styles: &'a [FormatSpan]) -> Option<&'a FormatSpan> {
    styles.iter().find(|span| span.range.start <= target.start && span.range.end >= target.end)
}

fn format_target(
    source: &str,
    target: &Range<usize>,
    styles: &[FormatSpan],
    marker: &str,
    remove: bool,
) -> String {
    let enclosing = enclosing_style(target, styles);
    if !remove && enclosing.is_some() {
        return source[target.clone()].to_owned();
    }
    let content = without_style_markers(source, target, styles);
    if remove {
        let opening = boundary_marker(source, target.start, styles);
        let closing = boundary_marker(source, target.end, styles);
        return format!("{opening}{content}{closing}");
    }
    let opening =
        if boundary_marker(source, target.start, styles).is_empty() { marker } else { "" };
    let closing = if boundary_marker(source, target.end, styles).is_empty() { marker } else { "" };
    format!("{opening}{content}{closing}")
}

fn boundary_marker<'a>(source: &'a str, boundary: usize, styles: &[FormatSpan]) -> &'a str {
    styles
        .iter()
        .find(|span| span.range.start < boundary && boundary < span.range.end)
        .map_or("", |span| &source[span.opening.clone()])
}

fn without_style_markers(source: &str, target: &Range<usize>, styles: &[FormatSpan]) -> String {
    let mut content = source[target.clone()].to_owned();
    let mut delimiters = Vec::new();
    for span in styles {
        for marker in [span.opening.clone(), span.closing.clone()] {
            if marker.start >= target.start && marker.end <= target.end {
                delimiters.push(marker);
            }
        }
    }
    delimiters.sort_by_key(|range| range.start);
    for marker in delimiters.into_iter().rev() {
        content.replace_range(marker.start - target.start..marker.end - target.start, "");
    }
    content
}
