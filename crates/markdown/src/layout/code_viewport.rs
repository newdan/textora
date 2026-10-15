//! Stateful horizontal viewports for code grids, applied before line publication.

use super::context::text_grapheme_x;
use super::types::{LaidOutBlock, LaidOutBlockKind, LaidOutLine};

const CARET_EDGE_INSET: f32 = 2.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct CodeViewport {
    source_start: usize,
    horizontal_offset: f32,
}

pub(super) fn update(
    blocks: &mut [Option<LaidOutBlock>],
    precise: &[bool],
    cursor: Option<usize>,
    previous: Option<CodeViewport>,
) -> (Option<CodeViewport>, Vec<usize>) {
    let mut next = previous.filter(|_| cursor.is_some());
    let mut changed_indices = Vec::new();
    for (index, (block, &is_precise)) in blocks.iter_mut().zip(precise).enumerate() {
        // Estimation must not overwrite a viewport measured with actual font advances.
        if !is_precise {
            continue;
        }
        let Some(block) = block else { continue };
        let (focused, changed) = update_block(block, cursor, previous);
        if focused.is_some() {
            next = focused;
        }
        if changed {
            changed_indices.push(index);
        }
    }
    (next, changed_indices)
}

fn update_block(
    block: &mut LaidOutBlock,
    cursor: Option<usize>,
    previous: Option<CodeViewport>,
) -> (Option<CodeViewport>, bool) {
    match &mut block.kind {
        LaidOutBlockKind::CodeBlock { lines, .. } => {
            update_grid(lines, block.rect.x, block.rect.w, cursor, previous)
        }
        LaidOutBlockKind::BlockQuote { blocks } | LaidOutBlockKind::ListItem { blocks, .. } => {
            let mut focused = None;
            let mut changed = false;
            for child in blocks {
                let (child_focused, child_changed) = update_block(child, cursor, previous);
                focused = child_focused.or(focused);
                changed |= child_changed;
            }
            (focused, changed)
        }
        _ => (None, false),
    }
}

fn update_grid(
    lines: &mut [LaidOutLine],
    block_left: f32,
    block_width: f32,
    cursor: Option<usize>,
    previous: Option<CodeViewport>,
) -> (Option<CodeViewport>, bool) {
    let Some(first) = lines.first() else { return (None, false) };
    // The right edge stays fixed even while the text origin moves horizontally.
    let padding = block_left + block_width - (first.rect.x + first.rect.w);
    let text_left = block_left + padding;
    let applied_offset = text_left - first.rect.x;
    let viewport_width = first.rect.w - applied_offset;
    let focused = cursor.and_then(|cursor| {
        let line = lines.iter().find(|line| {
            line.source_projection.as_ref().is_some_and(|projection| {
                projection.source_extent.start <= cursor && cursor <= projection.source_extent.end
            })
        })?;
        let source_start = first.source_projection.as_ref()?.source_extent.start;
        let local_byte = cursor - line.source_projection.as_ref()?.source_extent.start;
        let caret =
            line_advance(line, crate::grapheme_map::grapheme_index_at_byte(&line.text, local_byte));
        Some(CodeViewport {
            source_start,
            horizontal_offset: reveal_offset(lines, source_start, caret, viewport_width, previous),
        })
    });
    let offset = focused.map_or(0.0, |viewport| viewport.horizontal_offset);
    for line in lines {
        line.rect.x = text_left - offset;
        line.rect.w = viewport_width + offset;
    }
    (focused, (offset - applied_offset).abs() > f32::EPSILON)
}

fn reveal_offset(
    lines: &[LaidOutLine],
    source_start: usize,
    caret: f32,
    viewport_width: f32,
    previous: Option<CodeViewport>,
) -> f32 {
    let usable_width = (viewport_width - CARET_EDGE_INSET).max(0.0);
    let widest_line = lines
        .iter()
        .map(|line| line_advance(line, crate::grapheme_map::grapheme_count(&line.text)))
        .fold(0.0_f32, f32::max);
    let maximum_offset = (widest_line - usable_width).max(0.0);
    let offset = previous
        .filter(|viewport| viewport.source_start == source_start)
        .map_or(0.0, |viewport| viewport.horizontal_offset)
        .clamp(0.0, maximum_offset);
    if caret < offset {
        return caret;
    }
    offset.max(caret - usable_width)
}

fn line_advance(line: &LaidOutLine, grapheme: usize) -> f32 {
    text_grapheme_x(&line.text, line.shaped.as_ref(), line.font_size, grapheme)
}
