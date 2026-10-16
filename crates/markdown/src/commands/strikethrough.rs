use std::ops::Range;

use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd};
use ui::plugin::SemanticEditPlan;

const STRIKETHROUGH_MARKER: &str = "~~";

struct InlineRanges {
    blocks: Vec<Range<usize>>,
    strikes: Vec<Range<usize>>,
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
) -> SemanticEditPlan {
    let Some(selection) = selection.filter(|range| range.start < range.end) else {
        return super::plan_inline_toggle(
            source,
            source_generation,
            cursor_byte,
            None,
            STRIKETHROUGH_MARKER,
        );
    };
    let inline = inline_ranges(source);
    let targets: Vec<_> = inline
        .blocks
        .iter()
        .filter_map(|block| selected_content(source, block.clone(), &selection, &inline.strikes))
        .collect();
    if targets.is_empty() {
        return SemanticEditPlan::NoChange;
    }
    let remove = targets.iter().all(|target| fully_struck(source, target, &inline));
    let start = targets.first().expect("nonempty targets checked above").start;
    let end = targets.last().expect("nonempty targets checked above").end;
    let mut replacement = source[start..end].to_owned();
    for target in targets.iter().rev() {
        let formatted = format_target(source, target, &inline.strikes, remove);
        replacement.replace_range(target.start - start..target.end - start, &formatted);
    }
    if replacement == source[start..end] {
        return SemanticEditPlan::NoChange;
    }
    let cursor_after = start + replacement.len();
    super::apply_transaction(source_generation, start..end, replacement, cursor_after)
}

fn inline_ranges(source: &str) -> InlineRanges {
    let parsed = crate::parser::parse_markdown(source);
    let mut inline =
        InlineRanges { blocks: Vec::new(), strikes: Vec::new(), visible_content: Vec::new() };
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
                if matches!(event, MarkdownEvent::Start(MarkdownTag::Strikethrough)) {
                    inline.strikes.push(range.clone());
                }
                if !is_inline_content(event) {
                    continue;
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
        }
    }
    inline
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

fn fully_struck(source: &str, target: &Range<usize>, inline: &InlineRanges) -> bool {
    inline.visible_content.iter().all(|content| {
        let selected = content.start.max(target.start)..content.end.min(target.end);
        selected.start >= selected.end
            || source[selected.clone()].trim().is_empty()
            || enclosing_strike(&selected, &inline.strikes).is_some()
    })
}

fn selected_content(
    source: &str,
    block: Range<usize>,
    selection: &Range<usize>,
    strikes: &[Range<usize>],
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
    for strike in strikes {
        let marker_length = strike_marker_length(source, strike);
        if target.start >= strike.start && target.start <= strike.start + marker_length {
            target.start = strike.start;
        }
        if target.end >= strike.end - marker_length && target.end <= strike.end {
            target.end = strike.end;
        }
    }
    Some(target)
}

fn enclosing_strike<'a>(
    target: &Range<usize>,
    strikes: &'a [Range<usize>],
) -> Option<&'a Range<usize>> {
    strikes.iter().find(|strike| strike.start <= target.start && strike.end >= target.end)
}

fn strike_marker_length(source: &str, strike: &Range<usize>) -> usize {
    source[strike.clone()].bytes().take_while(|byte| *byte == b'~').count()
}

fn format_target(
    source: &str,
    target: &Range<usize>,
    strikes: &[Range<usize>],
    remove: bool,
) -> String {
    let enclosing = enclosing_strike(target, strikes);
    if !remove && enclosing.is_some() {
        return source[target.clone()].to_owned();
    }
    let content = without_strike_markers(source, target, strikes);
    if remove {
        let opening = boundary_marker(source, target.start, strikes);
        let closing = boundary_marker(source, target.end, strikes);
        return format!("{opening}{content}{closing}");
    }
    let opening = if boundary_marker(source, target.start, strikes).is_empty() {
        STRIKETHROUGH_MARKER
    } else {
        ""
    };
    let closing = if boundary_marker(source, target.end, strikes).is_empty() {
        STRIKETHROUGH_MARKER
    } else {
        ""
    };
    format!("{opening}{content}{closing}")
}

fn boundary_marker<'a>(source: &'a str, boundary: usize, strikes: &[Range<usize>]) -> &'a str {
    strikes
        .iter()
        .find(|strike| strike.start < boundary && boundary < strike.end)
        .map_or("", |strike| {
            &source[strike.start..strike.start + strike_marker_length(source, strike)]
        })
}

fn without_strike_markers(source: &str, target: &Range<usize>, strikes: &[Range<usize>]) -> String {
    let mut content = source[target.clone()].to_owned();
    let mut delimiters = Vec::new();
    for strike in strikes {
        let marker_length = strike_marker_length(source, strike);
        for marker in
            [strike.start..strike.start + marker_length, strike.end - marker_length..strike.end]
        {
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
