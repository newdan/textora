//! Render pass — converts LaidOutDoc into DrawList commands.

use std::sync::Arc;

use ui::core::geom::Rect;
use ui::core::paint::{DrawCmd, DrawList};
use ui::core::text_layout::UiTextLayout;

use crate::builder::{InlineStyle, ListBullet};
use crate::layout::block::MarkdownLayout;
use crate::layout::{
    AsciiDiagramRegistry, AsciiDiagramRow, BoxConnections, LaidOutBlock, LaidOutBlockKind,
    LaidOutDoc, LaidOutLine,
};
use crate::safe_byte_idx;
use crate::style::{MarkdownStyle, blend_toward_bg};
use shaping::{Style, Weight};
use ui::core::text_layout::ITALIC_SHEAR;

const SOURCE_MARKER_FADE_RATIO: f32 = 0.55;
const INLINE_CODE_BACKGROUND_HEIGHT_RATIO: f32 = 1.3;
const INLINE_CODE_BACKGROUND_RADIUS_RATIO: f32 = 0.35;
const INLINE_CODE_BACKGROUND_HORIZONTAL_PADDING_RATIO: f32 = 0.25;
const STRIKETHROUGH_THICKNESS_RATIO: f32 = 1.0 / 15.0;
const MIN_STRIKETHROUGH_THICKNESS: f32 = 1.0;

/// Render a laid-out markdown document into a DrawList.
///
/// `scroll_y` — vertical scroll offset in pixels.
/// `viewport_h` — visible height for clipping.
///
/// This compatibility API renders only [`LaidOutDoc`]. Use [`render_layout`]
/// with [`crate::layout::MarkdownLayout`] to retain optional Markdown render
/// sidecars such as ASCII diagram grids.
pub fn render_doc(
    doc: &LaidOutDoc,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    viewport_h: f32,
    shaper: Option<&mut shaping::Shaper>,
) {
    render_doc_with_offset(doc, style, dl, scroll_y, viewport_h, 0.0, 0.0, shaper, &[]);
}

/// Render a [`MarkdownLayout`] while preserving its Markdown render sidecars.
pub fn render_layout(
    layout: &MarkdownLayout,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    viewport_h: f32,
    shaper: Option<&mut shaping::Shaper>,
) {
    render_layout_with_offset(layout, style, dl, scroll_y, viewport_h, 0.0, 0.0, shaper, &[]);
}

/// Find the index of the first top-level block that might intersect [scroll_y, scroll_y+viewport_h].
///
/// **Precondition:** `blocks` must be sorted by `rect.y` ascending (guaranteed by layout).
/// `y_delta` is the cumulative y-offset correction array from LazyLayout.
pub fn first_visible_block_idx(blocks: &[LaidOutBlock], y_delta: &[f32], scroll_y: f32) -> usize {
    // Binary search on rect.y (always monotonic) to get a candidate neighborhood.
    let idx = blocks
        .binary_search_by(|b| {
            if b.rect.y + b.rect.h < scroll_y {
                std::cmp::Ordering::Less
            } else if b.rect.y > scroll_y {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .unwrap_or_else(|i| i.min(blocks.len().saturating_sub(1)));

    // Walk backward: y_delta may have pulled earlier blocks down into the visible range.
    // In practice this walks 0-2 blocks because cumulative y_delta is small relative
    // to block heights (typical estimation error is a few pixels per block).
    let mut result = idx;
    while result > 0 {
        let real_y = blocks[result - 1].rect.y + y_delta.get(result - 1).copied().unwrap_or(0.0);
        let real_bottom = real_y + blocks[result - 1].rect.h;
        if real_bottom >= scroll_y {
            result -= 1;
        } else {
            break;
        }
    }
    result
}

/// Render with pixel offset (used to position preview inside editor content area).
/// `y_delta` is the cumulative y-offset correction array from LazyLayout.
/// Pass `&[]` for non-lazy (full-precision) layouts.
pub fn render_doc_with_offset(
    doc: &LaidOutDoc,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    viewport_h: f32,
    offset_x: f32,
    offset_y: f32,
    shaper: Option<&mut shaping::Shaper>,
    y_delta: &[f32],
) {
    render_doc_with_offset_and_ascii_diagrams(
        doc, style, dl, scroll_y, viewport_h, offset_x, offset_y, shaper, y_delta, None,
    );
}

/// Render a [`MarkdownLayout`] with a pixel offset while preserving its sidecars.
pub fn render_layout_with_offset(
    layout: &MarkdownLayout,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    viewport_h: f32,
    offset_x: f32,
    offset_y: f32,
    shaper: Option<&mut shaping::Shaper>,
    y_delta: &[f32],
) {
    render_doc_with_offset_and_ascii_diagrams(
        layout.document(),
        style,
        dl,
        scroll_y,
        viewport_h,
        offset_x,
        offset_y,
        shaper,
        y_delta,
        Some(layout.ascii_diagrams()),
    );
}

pub(crate) fn render_doc_with_offset_and_ascii_diagrams(
    doc: &LaidOutDoc,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    viewport_h: f32,
    offset_x: f32,
    offset_y: f32,
    mut shaper: Option<&mut shaping::Shaper>,
    y_delta: &[f32],
    ascii_diagrams: Option<&AsciiDiagramRegistry>,
) {
    dl.cmds.push(DrawCmd::PushClip(Rect::new(offset_x, offset_y, f32::MAX, viewport_h)));

    let last_y = scroll_y + viewport_h;
    let start = first_visible_block_idx(&doc.blocks, y_delta, scroll_y);

    for i in start..doc.blocks.len() {
        let block = &doc.blocks[i];
        let real_y = block.rect.y + y_delta.get(i).copied().unwrap_or(0.0);
        if real_y > last_y {
            break;
        }
        render_block_with_offset(
            block,
            style,
            dl,
            scroll_y - y_delta.get(i).copied().unwrap_or(0.0),
            viewport_h,
            offset_x,
            offset_y,
            shaper.as_deref_mut(),
            ascii_diagrams,
        );
    }

    dl.cmds.push(DrawCmd::PopClip);
}

fn render_block_with_offset(
    block: &LaidOutBlock,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    viewport_h: f32,
    ox: f32,
    oy: f32,
    mut shaper: Option<&mut shaping::Shaper>,
    ascii_diagrams: Option<&AsciiDiagramRegistry>,
) {
    let r = block.rect;
    let x = r.x + ox;
    let y = r.y - scroll_y + oy;

    match &block.kind {
        LaidOutBlockKind::Text { lines } => {
            for line in lines {
                let line_bottom = line.rect.y + line.rect.h;
                if line_bottom < scroll_y {
                    continue;
                }
                if line.rect.y > scroll_y + viewport_h {
                    break;
                }
                render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
            }
        }
        LaidOutBlockKind::CodeBlock { lines, .. } => {
            // Background
            dl.fill_rounded(Rect::new(x, y, r.w, r.h), style.code_bg, style.border_radius_base);
            // Border
            dl.stroke_rounded(
                Rect::new(x, y, r.w, r.h),
                style.code_block_border,
                style.border_radius_base,
                1.0,
            );
            // Clipped code text
            dl.clip(Rect::new(x, y, r.w, r.h), |dl| {
                if let (Some(diagram), Some(shaper)) = (
                    ascii_diagrams.and_then(|registry| registry.diagram_for(lines)),
                    shaper.as_deref_mut(),
                ) && diagram.rows.len() == lines.len()
                {
                    let cell_width = code_cell_width(
                        shaper,
                        style.code_font_size,
                        style.code_font_family.as_deref(),
                    );
                    for (line, row) in lines.iter().zip(&diagram.rows) {
                        if line.rect.y + line.rect.h < scroll_y {
                            continue;
                        }
                        if line.rect.y > scroll_y + viewport_h {
                            break;
                        }
                        render_ascii_diagram_row(
                            line, row, cell_width, style, dl, scroll_y, ox, oy, shaper,
                        );
                    }
                } else {
                    for line in lines {
                        let line_bottom = line.rect.y + line.rect.h;
                        if line_bottom < scroll_y {
                            continue;
                        }
                        if line.rect.y > scroll_y + viewport_h {
                            break;
                        }
                        render_line_with_offset(
                            line,
                            style,
                            dl,
                            scroll_y,
                            ox,
                            oy,
                            shaper.as_deref_mut(),
                        );
                    }
                }
            });
        }
        LaidOutBlockKind::BlockQuote { blocks } => {
            // Painter's algorithm: background first (full width), then border on top
            dl.fill_rounded(
                Rect::new(x, y, r.w, r.h),
                style.blockquote_bg,
                style.border_radius_base,
            );
            dl.fill(Rect::new(x, y, 4.0, r.h), style.blockquote_border);
            // Children with culling
            for child in blocks {
                if child.rect.y + child.rect.h < scroll_y {
                    continue;
                }
                if child.rect.y > scroll_y + viewport_h {
                    break;
                }
                render_block_with_offset(
                    child,
                    style,
                    dl,
                    scroll_y,
                    viewport_h,
                    ox,
                    oy,
                    shaper.as_deref_mut(),
                    ascii_diagrams,
                );
            }
        }
        LaidOutBlockKind::ListItem { bullet, blocks, lines, level_indent: _, depth } => {
            let font_size = style.body_font_size;
            let bullet_x = x;
            let bullet_color = blend_toward_bg(style.text_color, style.background_color, 0.3);
            // Align bullet baseline with first text line, not block top.
            // Block rect.y may include list_item_spacing that the first text
            // line already incorporates, causing vertical misalignment.
            let bullet_baseline = lines.first().map_or(y, |l| l.rect.y - scroll_y + oy) + font_size;

            match bullet {
                ListBullet::Bullet => {
                    // Vary bullet symbol by nesting depth
                    let symbol = match depth % 3 {
                        0 => "•",
                        1 => "◦",
                        _ => "▪",
                    };
                    if let Some(ref mut s) = shaper {
                        dl.text_shaped(
                            bullet_x + 4.0,
                            bullet_baseline,
                            font_size,
                            bullet_color,
                            symbol,
                            s,
                        );
                    }
                }
                ListBullet::Ordered(_) if list_item_uses_source_marker(lines) => {}
                ListBullet::Ordered(n) => {
                    let label = format!("{}.", n);
                    if let Some(ref mut s) = shaper {
                        dl.text_shaped(
                            bullet_x + 4.0,
                            bullet_baseline,
                            font_size,
                            bullet_color,
                            &label,
                            s,
                        );
                    }
                }
                ListBullet::TaskList(checked) => {
                    let box_size = font_size * 0.75;
                    let box_x = bullet_x + 2.0;
                    // Center box on the same visual center as bullet text.
                    // bullet_baseline is at first_line_top + font_size (text baseline).
                    // Text visual center ≈ baseline - font_size * 0.4 (cap-height midpoint).
                    let box_y = bullet_baseline - font_size * 0.4 - box_size / 2.0;
                    // Unchecked: subtle border blended toward bg; Checked: full text color
                    let border_color = if *checked {
                        style.text_color
                    } else {
                        blend_toward_bg(style.text_color, style.background_color, 0.55)
                    };
                    dl.stroke_rounded(
                        Rect::new(box_x, box_y, box_size, box_size),
                        border_color,
                        3.0,
                        1.5,
                    );
                    if *checked {
                        let check_fs = box_size * 0.95;
                        let check_baseline = box_y + box_size / 2.0 + check_fs * 0.25;
                        if let Some(ref mut s) = shaper {
                            // Render at temporary x to measure real width, then center.
                            let idx = dl.cmds.len();
                            let actual_w = dl.text_shaped(
                                box_x,
                                check_baseline,
                                check_fs,
                                border_color,
                                "x",
                                s,
                            );
                            if let DrawCmd::TextLayout { ref mut x, .. } = dl.cmds[idx] {
                                *x = box_x + (box_size - actual_w) / 2.0;
                            }
                        }
                    }
                }
            }
            // Render item text lines
            for line in lines {
                let line_bottom = line.rect.y + line.rect.h;
                if line_bottom < scroll_y {
                    continue;
                }
                if line.rect.y > scroll_y + viewport_h {
                    break;
                }
                render_line_with_offset(line, style, dl, scroll_y, ox, oy, shaper.as_deref_mut());
            }
            // Render nested child blocks
            for child in blocks {
                if child.rect.y + child.rect.h < scroll_y {
                    continue;
                }
                if child.rect.y > scroll_y + viewport_h {
                    break;
                }
                render_block_with_offset(
                    child,
                    style,
                    dl,
                    scroll_y,
                    viewport_h,
                    ox,
                    oy,
                    shaper.as_deref_mut(),
                    ascii_diagrams,
                );
            }
        }
        LaidOutBlockKind::Table {
            columns,
            header,
            rows,
            column_widths,
            header_height,
            row_heights,
        } => {
            let mut cell_y = y;

            // Header — use actual measured height
            if !header.is_empty() && *header_height > 0.0 {
                dl.fill_rounded(
                    Rect::new(x, cell_y, r.w, *header_height),
                    style.table_header_bg,
                    0.0,
                );
                for cell_lines in header.iter() {
                    for line in cell_lines {
                        render_line_with_offset(
                            line,
                            style,
                            dl,
                            scroll_y,
                            ox,
                            oy,
                            shaper.as_deref_mut(),
                        );
                    }
                }
                cell_y += *header_height;
                // Separator line
                dl.fill(Rect::new(x, cell_y, r.w, 1.0), style.table_border);
            }

            // Body rows with zebra stripes — use actual measured heights
            for (row_idx, row) in rows.iter().enumerate() {
                let row_h = row_heights.get(row_idx).copied().unwrap_or(style.line_height + 2.0);
                // Zebra stripe: odd rows get a subtle background
                if row_idx % 2 == 1 {
                    dl.fill_rounded(Rect::new(x, cell_y, r.w, row_h), style.table_stripe_bg, 0.0);
                }
                for cell_lines in row.iter() {
                    for line in cell_lines {
                        render_line_with_offset(
                            line,
                            style,
                            dl,
                            scroll_y,
                            ox,
                            oy,
                            shaper.as_deref_mut(),
                        );
                    }
                }
                cell_y += row_h;
                // Row separator
                dl.fill(Rect::new(x, cell_y, r.w, 1.0), style.table_border);
            }

            // Vertical grid lines
            for i in 1..*columns {
                let cx = x + column_widths[..i].iter().sum::<f32>();
                dl.fill(Rect::new(cx, y, 1.0, r.h), style.table_border);
            }
        }
        LaidOutBlockKind::HorizontalRule => {
            let rule_w = r.w * style.rule_width_ratio;
            let rule_x = x + (r.w - rule_w) / 2.0;
            let rule_y = y + (r.h - style.rule_thickness) / 2.0;
            dl.fill(Rect::new(rule_x, rule_y, rule_w, style.rule_thickness), style.rule_color);
        }
        LaidOutBlockKind::MetadataBlock { lines } => {
            // Metadata blocks rendered like code blocks: background + border + clipped text
            dl.fill_rounded(Rect::new(x, y, r.w, r.h), style.code_bg, style.border_radius_base);
            dl.stroke_rounded(
                Rect::new(x, y, r.w, r.h),
                style.code_block_border,
                style.border_radius_base,
                1.0,
            );
            dl.clip(Rect::new(x, y, r.w, r.h), |dl| {
                for line in lines {
                    let line_bottom = line.rect.y + line.rect.h;
                    if line_bottom < scroll_y {
                        continue;
                    }
                    if line.rect.y > scroll_y + viewport_h {
                        break;
                    }
                    render_line_with_offset(
                        line,
                        style,
                        dl,
                        scroll_y,
                        ox,
                        oy,
                        shaper.as_deref_mut(),
                    );
                }
            });
        }
    }
}

fn list_item_uses_source_marker(lines: &[LaidOutLine]) -> bool {
    lines.first().is_some_and(|line| {
        line.styles
            .iter()
            .any(|style| style.start == 0 && matches!(style.style, InlineStyle::SourceMarker))
    })
}

fn code_cell_width(shaper: &mut shaping::Shaper, font_size: f32, family: Option<&str>) -> f32 {
    let old_size = shaper.font_size();
    let old_family = shaper.font_family().map(str::to_owned);
    shaper.set_font_size(font_size);
    shaper.set_font_family(family);
    let width = shaper.col_width();
    shaper.set_font_size(old_size);
    shaper.set_font_family(old_family.as_deref());
    width
}

fn draw_box_connections(
    connections: BoxConnections,
    cell_x: f32,
    left_extension_width: f32,
    line_top: f32,
    cell_width: f32,
    line_height: f32,
    font_size: f32,
    color: [f32; 4],
    dl: &mut DrawList,
) {
    let thickness = (font_size * 0.08).clamp(1.0, 2.0);
    let center_x = cell_x + cell_width * 0.5;
    let center_y = line_top + line_height * 0.5;
    let half = thickness * 0.5;

    if connections.left {
        dl.fill(
            Rect::new(
                cell_x - left_extension_width,
                center_y - half,
                left_extension_width + cell_width * 0.5 + half,
                thickness,
            ),
            color,
        );
    }
    if connections.right {
        dl.fill(
            Rect::new(center_x - half, center_y - half, cell_width * 0.5 + half, thickness),
            color,
        );
    }
    if connections.up {
        dl.fill(Rect::new(center_x - half, line_top, thickness, line_height * 0.5 + half), color);
    }
    if connections.down {
        dl.fill(
            Rect::new(center_x - half, center_y - half, thickness, line_height * 0.5 + half),
            color,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HorizontalArrowDirection {
    Left,
    Right,
}

fn horizontal_arrow_direction(
    row: &AsciiDiagramRow,
    cell_index: usize,
) -> Option<HorizontalArrowDirection> {
    let cell = row.cells.get(cell_index)?;
    match cell.text.as_str() {
        "→" if cell_index
            .checked_sub(1)
            .and_then(|index| row.cells.get(index))
            .is_some_and(|neighbor| neighbor.text == "─") =>
        {
            Some(HorizontalArrowDirection::Right)
        }
        "←" if row.cells.get(cell_index + 1).is_some_and(|neighbor| neighbor.text == "─") => {
            Some(HorizontalArrowDirection::Left)
        }
        _ => None,
    }
}

fn draw_horizontal_arrow(
    direction: HorizontalArrowDirection,
    cell_x: f32,
    line_top: f32,
    cell_width: f32,
    line_height: f32,
    font_size: f32,
    color: [f32; 4],
    draw_list: &mut DrawList,
) {
    const TIP_INSET_RATIO: f32 = 0.1;
    const HEAD_LENGTH_RATIO: f32 = 0.45;
    const HEAD_HALF_HEIGHT_FONT_RATIO: f32 = 0.28;
    const MAXIMUM_HEAD_HALF_LINE_RATIO: f32 = 0.4;

    let thickness = (font_size * 0.08).clamp(1.0, 2.0);
    let half_thickness = thickness * 0.5;
    let center_y = line_top + line_height * 0.5;
    let head_half_height =
        (font_size * HEAD_HALF_HEIGHT_FONT_RATIO).min(line_height * MAXIMUM_HEAD_HALF_LINE_RATIO);
    let tip_inset = cell_width * TIP_INSET_RATIO;
    let head_length = cell_width * HEAD_LENGTH_RATIO;
    let (tip_x, base_x, shaft_left, shaft_right) = match direction {
        HorizontalArrowDirection::Left => {
            let tip_x = cell_x + tip_inset;
            let base_x = tip_x + head_length;
            (tip_x, base_x, base_x - half_thickness, cell_x + cell_width)
        }
        HorizontalArrowDirection::Right => {
            let tip_x = cell_x + cell_width - tip_inset;
            let base_x = tip_x - head_length;
            (tip_x, base_x, cell_x, base_x + half_thickness)
        }
    };

    draw_list.fill(
        Rect::new(shaft_left, center_y - half_thickness, shaft_right - shaft_left, thickness),
        color,
    );
    draw_list.fill_triangle(
        [tip_x, center_y],
        [base_x, center_y - head_half_height],
        [base_x, center_y + head_half_height],
        color,
    );
}

fn render_ascii_diagram_row(
    line: &LaidOutLine,
    row: &AsciiDiagramRow,
    cell_width: f32,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    ox: f32,
    oy: f32,
    shaper: &mut shaping::Shaper,
) {
    let line_top = line.rect.y - scroll_y + oy;
    let base_color = line.color_override.unwrap_or(style.text_color);
    let font_family = style.code_font_family.clone();
    let mut cell_byte_start = 0usize;

    for (cell_index, cell) in row.cells.iter().enumerate() {
        let cell_byte_end = cell_byte_start + cell.text.len();
        if cell.text.trim().is_empty() {
            cell_byte_start = cell_byte_end;
            continue;
        }

        let render_column = cell.render_column();
        let cell_x = line.rect.x + ox + render_column as f32 * cell_width;
        let allocated_width = cell.column_width as f32 * cell_width;
        let left_extension_width = cell.left_extension_columns() as f32 * cell_width;
        let color = highlight_color_for_cell(&line.highlight_spans, cell_byte_start, cell_byte_end)
            .unwrap_or(base_color);
        if let Some(connections) = cell.box_connections {
            draw_box_connections(
                connections,
                cell_x,
                left_extension_width,
                line_top,
                allocated_width,
                line.rect.h,
                line.font_size,
                color,
                dl,
            );
            cell_byte_start = cell_byte_end;
            continue;
        }

        if let Some(direction) = horizontal_arrow_direction(row, cell_index) {
            draw_horizontal_arrow(
                direction,
                cell_x,
                line_top,
                allocated_width,
                line.rect.h,
                line.font_size,
                color,
                dl,
            );
            cell_byte_start = cell_byte_end;
            continue;
        }

        let Some(layout) = UiTextLayout::new(
            &cell.text,
            line.font_size,
            font_family.clone(),
            line.font_weight,
            Style::Normal,
            false,
            shaper,
        ) else {
            cell_byte_start = cell_byte_end;
            continue;
        };
        let text_x = cell_x + (allocated_width - layout.shaped.width) * 0.5;
        dl.text_layout(Arc::new(layout), text_x, line_top + line.font_size, color);
        cell_byte_start = cell_byte_end;
    }
}

fn highlight_color_for_cell(
    highlight_spans: &[crate::builder::HighlightSpan],
    cell_byte_start: usize,
    cell_byte_end: usize,
) -> Option<[f32; 4]> {
    highlight_spans
        .iter()
        .find(|span| {
            let span_end = span.start.saturating_add(span.len);
            span.start < cell_byte_end && cell_byte_start < span_end
        })
        .map(|span| span.color)
}

fn render_line_with_offset(
    line: &LaidOutLine,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    ox: f32,
    oy: f32,
    mut shaper: Option<&mut shaping::Shaper>,
) {
    let ly = line.rect.y - scroll_y + oy;
    let base_color = line.color_override.unwrap_or(style.text_color);
    let font_size = line.font_size;
    let line_x = line.rect.x + ox;

    let font_family = if line.is_code {
        style.code_font_family.clone()
    } else {
        style.body_font_family.first().cloned()
    };

    // Fast path: no inline styles at all
    if line.styles.is_empty() {
        if !line.highlight_spans.is_empty() {
            // Syntax-highlighted code line: render per-span with highlight colors
            // Use actual HarfBuzz shaped widths to advance cursor (not estimated widths)
            let text_len = line.text.len();
            let mut cursor_x = line_x;
            let mut last_end = 0usize;
            for span in &line.highlight_spans {
                let span_start = span.start.min(text_len);
                let span_end = (span.start + span.len).min(text_len);
                if span_start > last_end {
                    // Gap before this span — render with base color
                    let gap = &line.text[safe_byte_idx(&line.text, last_end)
                        ..safe_byte_idx(&line.text, span_start)];
                    if let Some(ref mut s) = shaper {
                        let w = dl.text_shaped_with_font(
                            cursor_x,
                            ly + font_size,
                            font_size,
                            base_color,
                            gap,
                            font_family.clone(),
                            line.font_weight,
                            Style::Normal,
                            false,
                            s,
                        );
                        cursor_x += w;
                    } else {
                        cursor_x += estimate_text_width(gap, font_size);
                    }
                }
                if span_start < text_len {
                    let segment = &line.text[safe_byte_idx(&line.text, span_start)
                        ..safe_byte_idx(&line.text, span_end)];
                    if let Some(ref mut s) = shaper {
                        let w = dl.text_shaped_with_font(
                            cursor_x,
                            ly + font_size,
                            font_size,
                            span.color,
                            segment,
                            font_family.clone(),
                            line.font_weight,
                            Style::Normal,
                            false,
                            s,
                        );
                        cursor_x += w;
                    } else {
                        cursor_x += estimate_text_width(segment, font_size);
                    }
                }
                last_end = span_end;
            }
            if last_end < text_len {
                let tail = &line.text[safe_byte_idx(&line.text, last_end)..];
                if let Some(ref mut s) = shaper {
                    dl.text_shaped_with_font(
                        cursor_x,
                        ly + font_size,
                        font_size,
                        base_color,
                        tail,
                        font_family,
                        line.font_weight,
                        Style::Normal,
                        false,
                        s,
                    );
                }
            }
        } else if let Some(ref layout) = line.text_layout {
            dl.text_layout(layout.clone(), line_x, ly + font_size, base_color);
        } else if let Some(ref mut s) = shaper {
            dl.text_shaped_with_font(
                line_x,
                ly + font_size,
                font_size,
                base_color,
                &line.text,
                font_family,
                line.font_weight,
                Style::Normal,
                false,
                s,
            );
        }
        return;
    }

    let text_len = line.text.len();

    // If no precomputed style_segments, fall back to estimated positions
    if line.style_segments.is_empty() {
        // Draw inline code backgrounds
        for span in &line.styles {
            if span.start >= text_len {
                continue;
            }
            if matches!(span.style, InlineStyle::InlineCode) {
                let span_end = (span.start + span.len).min(text_len);
                let segment = &line.text
                    [safe_byte_idx(&line.text, span.start)..safe_byte_idx(&line.text, span_end)];
                let w = measure_text_width_with_font(
                    segment,
                    font_size,
                    font_family.as_deref(),
                    line.font_weight,
                    Style::Normal,
                    shaper.as_deref_mut(),
                );
                let prefix = &line.text[..safe_byte_idx(&line.text, span.start)];
                let prefix_w = measure_text_width_with_font(
                    prefix,
                    font_size,
                    font_family.as_deref(),
                    line.font_weight,
                    Style::Normal,
                    shaper.as_deref_mut(),
                );
                let padding =
                    inline_code_background_padding(&line.text, span.start, span_end, font_size);
                let rect =
                    inline_code_background_rect(line_x + prefix_w, ly, w, font_size, padding);
                draw_inline_code_bg(style, dl, rect);
            }
        }
        // Render text split by style spans using actual shaped widths
        let mut cursor_x = line_x;
        let mut last_end = 0usize;
        for span in &line.styles {
            if span.start >= text_len {
                continue;
            }
            if span.start > last_end {
                let gap = &line.text
                    [safe_byte_idx(&line.text, last_end)..safe_byte_idx(&line.text, span.start)];
                if let Some(ref mut s) = shaper {
                    let w = dl.text_shaped_with_font(
                        cursor_x,
                        ly + font_size,
                        font_size,
                        base_color,
                        gap,
                        font_family.clone(),
                        line.font_weight,
                        Style::Normal,
                        false,
                        s,
                    );
                    cursor_x += w;
                } else {
                    cursor_x += estimate_text_width(gap, font_size);
                }
            }
            let span_end = (span.start + span.len).min(text_len);
            if span.start < text_len {
                let segment = &line.text
                    [safe_byte_idx(&line.text, span.start)..safe_byte_idx(&line.text, span_end)];
                let color = style_for_span(&span.style, base_color, style);
                let mut w = if let Some(ref mut s) = shaper {
                    if needs_styled_text(&span.style) {
                        let (ws_weight, ws_style) = weight_style_for(&span.style);
                        let italic = is_italic(&span.style);
                        dl.text_shaped_with_font(
                            cursor_x,
                            ly + font_size,
                            font_size,
                            color,
                            segment,
                            font_family.clone(),
                            ws_weight,
                            ws_style,
                            italic,
                            s,
                        )
                    } else {
                        dl.text_shaped_with_font(
                            cursor_x,
                            ly + font_size,
                            font_size,
                            color,
                            segment,
                            font_family.clone(),
                            line.font_weight,
                            Style::Normal,
                            false,
                            s,
                        )
                    }
                } else {
                    estimate_text_width(segment, font_size)
                };
                if is_italic(&span.style) {
                    w += font_size * ITALIC_SHEAR;
                }
                if is_underlined(&span.style) {
                    dl.fill(Rect::new(cursor_x, ly + font_size + 2.0, w, 1.0), color);
                }
                if is_strikethrough(&span.style) {
                    dl.fill(
                        Rect::new(
                            cursor_x,
                            ly + font_size * 0.55,
                            w,
                            strikethrough_thickness(font_size),
                        ),
                        color,
                    );
                }
                cursor_x += w;
            }
            last_end = span_end;
        }
        if last_end < text_len {
            let tail = &line.text[safe_byte_idx(&line.text, last_end)..];
            if let Some(ref mut s) = shaper {
                dl.text_shaped_with_font(
                    cursor_x,
                    ly + font_size,
                    font_size,
                    base_color,
                    tail,
                    font_family,
                    line.font_weight,
                    Style::Normal,
                    false,
                    s,
                );
            }
        }
        return;
    }

    // Use precomputed style_segments for precise positioning
    let segments = &line.style_segments;

    // 1) Draw inline code backgrounds (behind text)
    for seg in segments {
        if matches!(seg.style, InlineStyle::InlineCode) {
            let seg_end = (seg.start + seg.len).min(text_len);
            let padding = inline_code_background_padding(&line.text, seg.start, seg_end, font_size);
            let rect = inline_code_background_rect(
                line_x + seg.x_offset,
                ly,
                seg.width,
                font_size,
                padding,
            );
            draw_inline_code_bg(style, dl, rect);
        }
    }

    // 2) Render text in segment order: gap, styled, gap, styled, tail
    let mut last_end = 0usize;

    for seg in segments {
        if seg.start >= text_len {
            continue;
        }
        // Render unstyled gap before this segment
        if seg.start > last_end {
            let gap = &line.text
                [safe_byte_idx(&line.text, last_end)..safe_byte_idx(&line.text, seg.start)];
            let gap_x = end_x_for_offset(segments, line_x, last_end, text_len);
            if let Some(ref mut s) = shaper {
                dl.text_shaped_with_font(
                    gap_x,
                    ly + font_size,
                    font_size,
                    base_color,
                    gap,
                    font_family.clone(),
                    line.font_weight,
                    Style::Normal,
                    false,
                    s,
                );
            }
        }
        // Render styled segment
        let seg_end = (seg.start + seg.len).min(text_len);
        if seg.start < text_len {
            let segment = &line.text
                [safe_byte_idx(&line.text, seg.start)..safe_byte_idx(&line.text, seg_end)];
            let color = style_for_span(&seg.style, base_color, style);
            let x = line_x + seg.x_offset;
            if let Some(ref mut s) = shaper {
                if needs_styled_text(&seg.style) {
                    let (ws_weight, ws_style) = weight_style_for(&seg.style);
                    let italic = is_italic(&seg.style);
                    dl.text_shaped_with_font(
                        x,
                        ly + font_size,
                        font_size,
                        color,
                        segment,
                        font_family.clone(),
                        ws_weight,
                        ws_style,
                        italic,
                        s,
                    );
                } else {
                    dl.text_shaped_with_font(
                        x,
                        ly + font_size,
                        font_size,
                        color,
                        segment,
                        font_family.clone(),
                        line.font_weight,
                        Style::Normal,
                        false,
                        s,
                    );
                }
            }
            let mut seg_w = seg.width;
            if is_italic(&seg.style) {
                seg_w += font_size * ITALIC_SHEAR;
            }
            if is_underlined(&seg.style) {
                dl.fill(Rect::new(x, ly + font_size + 2.0, seg_w, 1.0), color);
            }
            if is_strikethrough(&seg.style) {
                dl.fill(
                    Rect::new(x, ly + font_size * 0.55, seg_w, strikethrough_thickness(font_size)),
                    color,
                );
            }
        }
        last_end = seg_end;
    }

    // Render remaining unstyled tail after last segment
    if last_end < text_len {
        let tail = &line.text[safe_byte_idx(&line.text, last_end)..];
        let tail_x = end_x_for_offset(segments, line_x, last_end, text_len);
        if let Some(ref mut s) = shaper {
            dl.text_shaped_with_font(
                tail_x,
                ly + font_size,
                font_size,
                base_color,
                tail,
                font_family,
                line.font_weight,
                Style::Normal,
                false,
                s,
            );
        }
    }
}

fn inline_code_background_padding(
    line_text: &str,
    span_start: usize,
    span_end: usize,
    font_size: f32,
) -> (f32, f32) {
    let padding = font_size * INLINE_CODE_BACKGROUND_HORIZONTAL_PADDING_RATIO;
    let left_padding = if line_text[..safe_byte_idx(line_text, span_start)]
        .chars()
        .next_back()
        .is_some_and(char::is_whitespace)
    {
        padding
    } else {
        0.0
    };
    let right_padding = if line_text[safe_byte_idx(line_text, span_end)..]
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
    {
        padding
    } else {
        0.0
    };

    (left_padding, right_padding)
}

fn inline_code_background_rect(
    text_x: f32,
    line_y: f32,
    text_width: f32,
    font_size: f32,
    padding: (f32, f32),
) -> Rect {
    let (left_padding, right_padding) = padding;
    Rect::new(
        text_x - left_padding,
        line_y,
        text_width + left_padding + right_padding,
        font_size * INLINE_CODE_BACKGROUND_HEIGHT_RATIO,
    )
}

fn draw_inline_code_bg(style: &MarkdownStyle, dl: &mut DrawList, rect: Rect) {
    let radius = rect.h * INLINE_CODE_BACKGROUND_RADIUS_RATIO;
    dl.fill_rounded(rect, style.inline_code_bg, radius);
}

fn measure_text_width_with_font(
    text: &str,
    font_size: f32,
    font_family: Option<&str>,
    font_weight: Weight,
    font_style: Style,
    shaper: Option<&mut shaping::Shaper>,
) -> f32 {
    let Some(shaper) = shaper else {
        return estimate_text_width(text, font_size);
    };
    if text.is_empty() {
        return 0.0;
    }

    let old_size = shaper.font_size();
    let old_weight = shaper.font_weight();
    let old_style = shaper.font_style();
    let old_family = shaper.font_family().map(str::to_string);

    shaper.set_font_size(font_size);
    shaper.set_font_weight(font_weight);
    shaper.set_font_style(font_style);
    shaper.set_font_family(font_family);
    let width = shaper
        .shape(text)
        .map(|run| run.width)
        .unwrap_or_else(|_| estimate_text_width(text, font_size));

    shaper.set_font_size(old_size);
    shaper.set_font_weight(old_weight);
    shaper.set_font_style(old_style);
    shaper.set_font_family(old_family.as_deref());

    width
}

/// Compute the x position at a given byte offset within a line.
/// Uses the nearest preceding style_segment's position + its width.
fn end_x_for_offset(
    segments: &[crate::layout::StyleSegment],
    line_x: f32,
    byte_offset: usize,
    text_len: usize,
) -> f32 {
    if byte_offset == 0 {
        return line_x;
    }
    // Find the segment that ends at or before byte_offset
    for seg in segments.iter().rev() {
        let seg_end = (seg.start + seg.len).min(text_len);
        if seg_end <= byte_offset {
            return line_x + seg.x_offset + seg.width;
        }
    }
    line_x
}

/// Get the rendering color for an inline style.
fn style_for_span(inline: &InlineStyle, base_color: [f32; 4], style: &MarkdownStyle) -> [f32; 4] {
    match inline {
        InlineStyle::Bold => base_color,
        InlineStyle::Italic => base_color,
        InlineStyle::Strikethrough => base_color,
        InlineStyle::InlineCode => style.code_color,
        InlineStyle::Link { .. } => style.link_color,
        InlineStyle::SourceMarker => {
            blend_toward_bg(base_color, style.background_color, SOURCE_MARKER_FADE_RATIO)
        }
    }
}

fn is_underlined(inline: &InlineStyle) -> bool {
    matches!(inline, InlineStyle::Link { .. })
}

fn is_strikethrough(inline: &InlineStyle) -> bool {
    matches!(inline, InlineStyle::Strikethrough)
}

fn strikethrough_thickness(font_size: f32) -> f32 {
    (font_size * STRIKETHROUGH_THICKNESS_RATIO).max(MIN_STRIKETHROUGH_THICKNESS)
}

/// Map an inline style to (font_weight, font_style).
/// Note: Italic returns Style::Normal — slant is applied in vertex stage via shear transform.
/// Bold uses SEMIBOLD (600) instead of BOLD (700) because macOS CJK fonts
/// (PingFang SC) max out at Semibold; BOLD queries trigger cross-font fallback.
fn weight_style_for(inline: &InlineStyle) -> (Weight, Style) {
    match inline {
        InlineStyle::Bold => (Weight::SEMIBOLD, Style::Normal),
        InlineStyle::Italic => (Weight::NORMAL, Style::Normal),
        InlineStyle::SourceMarker
        | InlineStyle::Strikethrough
        | InlineStyle::InlineCode
        | InlineStyle::Link { .. } => (Weight::NORMAL, Style::Normal),
    }
}

fn is_italic(inline: &InlineStyle) -> bool {
    matches!(inline, InlineStyle::Italic)
}

/// Whether this inline style needs a non-default weight or style.
fn needs_styled_text(inline: &InlineStyle) -> bool {
    matches!(inline, InlineStyle::Bold | InlineStyle::Italic)
}

/// Text width estimate, CJK-aware.
///
/// CJK/fullwidth characters occupy roughly 1.0 × font_size;
/// ASCII and other narrow characters occupy roughly 0.55 × font_size.
fn estimate_text_width(text: &str, font_size: f32) -> f32 {
    let mut w = 0.0f32;
    for ch in text.chars() {
        if crate::layout::is_cjk_or_fullwidth(ch) {
            w += font_size;
        } else {
            w += font_size * 0.55;
        }
    }
    w
}

fn heading_spacing_scale(level: u8) -> f32 {
    if level <= 1 {
        1.0
    } else if level <= 3 {
        0.8
    } else {
        0.65
    }
}

/// Debug visualization: draw colored rectangles showing top/bottom spacing of each block.
/// Each element type gets a unique color. Top spacing = solid, bottom spacing = semi-transparent.
pub fn render_debug_spacing(
    doc: &LaidOutDoc,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    viewport_h: f32,
    offset_x: f32,
    offset_y: f32,
    y_delta: &[f32],
    shaper: &mut shaping::Shaper,
) {
    use LaidOutBlockKind::*;

    // Color palette for different element types (RGBA)
    const H1_TOP: [f32; 4] = [0.1, 0.4, 0.9, 0.9];
    const H1_BOT: [f32; 4] = [0.1, 0.4, 0.9, 0.5];
    const H2_TOP: [f32; 4] = [0.2, 0.55, 1.0, 0.85];
    const H2_BOT: [f32; 4] = [0.2, 0.55, 1.0, 0.45];
    const H3_TOP: [f32; 4] = [0.35, 0.65, 1.0, 0.8];
    const H3_BOT: [f32; 4] = [0.35, 0.65, 1.0, 0.4];
    const H4_TOP: [f32; 4] = [0.5, 0.75, 1.0, 0.75];
    const H4_BOT: [f32; 4] = [0.5, 0.75, 1.0, 0.4];
    const H5_TOP: [f32; 4] = [0.6, 0.82, 1.0, 0.7];
    const H5_BOT: [f32; 4] = [0.6, 0.82, 1.0, 0.35];
    const H6_TOP: [f32; 4] = [0.7, 0.88, 1.0, 0.65];
    const H6_BOT: [f32; 4] = [0.7, 0.88, 1.0, 0.3];
    const PARA_TOP: [f32; 4] = [0.3, 0.8, 0.3, 0.8];
    const PARA_BOT: [f32; 4] = [0.3, 0.8, 0.3, 0.4];
    const CODE_TOP: [f32; 4] = [0.9, 0.5, 0.2, 0.8];
    const CODE_BOT: [f32; 4] = [0.9, 0.5, 0.2, 0.4];
    const LIST_TOP: [f32; 4] = [0.8, 0.3, 0.8, 0.8];
    const LIST_BOT: [f32; 4] = [0.8, 0.3, 0.8, 0.4];
    const QUOTE_TOP: [f32; 4] = [0.9, 0.9, 0.2, 0.8];
    const QUOTE_BOT: [f32; 4] = [0.9, 0.9, 0.2, 0.4];
    const TABLE_TOP: [f32; 4] = [0.2, 0.8, 0.8, 0.8];
    const TABLE_BOT: [f32; 4] = [0.2, 0.8, 0.8, 0.4];
    const RULE_TOP: [f32; 4] = [0.8, 0.2, 0.2, 0.8];
    const RULE_BOT: [f32; 4] = [0.8, 0.2, 0.2, 0.4];
    const META_TOP: [f32; 4] = [0.5, 0.5, 0.5, 0.8];
    const META_BOT: [f32; 4] = [0.5, 0.5, 0.5, 0.4];
    const LABEL_COLOR: [f32; 4] = [0.95, 0.5, 0.15, 1.0];
    let label_font_size = style.body_font_size * 0.85;

    fn heading_colors(level: u8) -> ([f32; 4], [f32; 4]) {
        match level {
            1 => (H1_TOP, H1_BOT),
            2 => (H2_TOP, H2_BOT),
            3 => (H3_TOP, H3_BOT),
            4 => (H4_TOP, H4_BOT),
            5 => (H5_TOP, H5_BOT),
            _ => (H6_TOP, H6_BOT),
        }
    }

    fn is_heading_text(lines: &[LaidOutLine], style: &MarkdownStyle) -> bool {
        if let Some(first_line) = lines.first() {
            return first_line.font_size > style.body_font_size * 1.1;
        }
        false
    }

    fn detect_heading_level(lines: &[LaidOutLine], style: &MarkdownStyle) -> u8 {
        if let Some(first_line) = lines.first() {
            let font_size = first_line.font_size;
            for (i, &size) in style.heading_font_sizes.iter().enumerate() {
                if (font_size - size).abs() < 1.0 {
                    return (i + 1) as u8;
                }
            }
        }
        1
    }

    let last_y = scroll_y + viewport_h;
    let start = first_visible_block_idx(&doc.blocks, y_delta, scroll_y);

    let mut prev_bottom: Option<f32> = None;
    let mut prev_trailing: f32 = 0.0;
    let mut prev_was_heading: bool = false;

    for i in start..doc.blocks.len() {
        let block = &doc.blocks[i];
        let real_y = block.rect.y + y_delta.get(i).copied().unwrap_or(0.0);
        if real_y > last_y {
            break;
        }

        let r = block.rect;
        let x = r.x + offset_x;
        let y = real_y - scroll_y + offset_y;

        let (top_color, bottom_color, current_leading, bottom_spacing, label, is_heading) =
            match &block.kind {
                Text { lines } => {
                    if is_heading_text(lines, style) {
                        let level = detect_heading_level(lines, style);
                        let scale = heading_spacing_scale(level);
                        let (h_top, h_bot) = heading_colors(level);
                        let desired_top = style.heading_spacing_top * scale;
                        let leading = if prev_bottom.is_none() {
                            desired_top * 0.5
                        } else if prev_was_heading {
                            (desired_top - prev_trailing).max(0.0)
                        } else {
                            desired_top
                        };
                        (
                            h_top,
                            h_bot,
                            leading,
                            style.heading_spacing_bottom,
                            format!("H{}", level),
                            true,
                        )
                    } else {
                        (
                            PARA_TOP,
                            PARA_BOT,
                            0.0,
                            style.paragraph_spacing,
                            "Para".to_string(),
                            false,
                        )
                    }
                }
                CodeBlock { .. } => {
                    (CODE_TOP, CODE_BOT, 0.0, style.paragraph_spacing, "Code".to_string(), false)
                }
                BlockQuote { .. } => {
                    (QUOTE_TOP, QUOTE_BOT, 0.0, style.paragraph_spacing, "Quote".to_string(), false)
                }
                ListItem { .. } => {
                    (LIST_TOP, LIST_BOT, 0.0, style.list_item_spacing, "List".to_string(), false)
                }
                Table { .. } => {
                    (TABLE_TOP, TABLE_BOT, 0.0, style.paragraph_spacing, "Table".to_string(), false)
                }
                HorizontalRule => (RULE_TOP, RULE_BOT, 0.0, 0.0, "HR".to_string(), false),
                MetadataBlock { .. } => {
                    (META_TOP, META_BOT, 0.0, style.paragraph_spacing, "Meta".to_string(), false)
                }
            };

        let expected_top = prev_trailing + current_leading;
        let actual_top = if let Some(prev_b) = prev_bottom { (y - prev_b).max(0.0) } else { y };

        let top_spacing = actual_top;

        if top_spacing > 0.5 {
            dl.fill(Rect::new(x, y - top_spacing, r.w, top_spacing), top_color);
            let label_text = if (actual_top - expected_top).abs() > 1.0 && expected_top > 0.5 {
                format!("{} {:.0}/{:.0}", label, actual_top, expected_top)
            } else {
                format!("{} {:.0}", label, actual_top)
            };
            let label_y = y - top_spacing + label_font_size + 2.0;
            dl.text_shaped(x + 4.0, label_y, label_font_size, LABEL_COLOR, &label_text, shaper);
        }

        if bottom_spacing > 0.5 {
            dl.fill(Rect::new(x, y + r.h, r.w, bottom_spacing), bottom_color);
            let bot_label = format!("{:.0}", bottom_spacing);
            let bot_label_y = y + r.h + label_font_size + 2.0;
            dl.text_shaped(x + 4.0, bot_label_y, label_font_size, LABEL_COLOR, &bot_label, shaper);
        }

        dl.stroke(Rect::new(x, y, r.w, r.h), [1.0, 1.0, 1.0, 0.15], 0.5);

        prev_bottom = Some(y + r.h);
        prev_trailing = bottom_spacing;
        prev_was_heading = is_heading;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{CodeHighlighter, HighlightSpan, MarkdownDoc};
    use crate::layout::block::{MarkdownLayout, layout_doc_with_shaper_for_rendering};
    use crate::layout::{LaidOutBlockKind, LaidOutDoc, LazyLayout, layout_doc};
    use crate::parser::parse_markdown;

    use crate::test_utils::default_style;

    const ELECTRON_GO_ARCHITECTURE_DIAGRAM: &str = r#"```
Electron (UI Shell)          Go (Agent Core)
┌──────────────────────┐    ┌─────────────────────────────┐
│  Main Process         │    │  WebSocket Server            │
│  ├─ spawn Go 二进制    │◄──►│  ├─ token 认证               │
│  └─ BrowserWindow     │ WS │  └─ 收发 JSON 消息           │
│                       │    │                              │
│  Renderer (Chat UI)   │    │  Agent Loop                  │
│  ├─ 流式对话           │    │  ├─ Orchestrator (主 agent)  │
│  ├─ 工具调用卡片        │    │  └─ Worker (子 agent)       │
│  ├─ 子 agent 进度      │    │                              │
│  └─ 任务面板            │    │  LLM Provider               │
└──────────────────────┘    │  ├─ Anthropic (流式 SSE)      │
                            │  └─ OpenAI 兼容               │
                            │                               │
                            │  工具系统 (8 tools)            │
                            │  条件式 Prompt 构建            │
                            │  Skills 系统                  │
                            │  会话持久化                    │
                            └─────────────────────────────┘
```"#;

    const WPS_ARCHITECTURE_DIAGRAM: &str = r#"```
┌──────────── WPS 客户端 ────────────┐          ┌────── 服务端（增量持续）──────┐
│                                     │          │                              │
│  ┌─ 本地日志（30天滚动）─────────┐  │  每日增量 │  ┌─ 画像更新 Pipeline ───┐  │
│  │ · 文件操作（打开/关闭/保存）  │  │  上传     │  │ · 角色标签更新          │  │
│  │ · 编辑会话（时长/段落/光标）  │  │────────→ │  │ · 模板偏好更新          │  │
│  │ · 模板使用记录               │  │  DSL包   │  │ · 活跃项目检测          │  │
│  │ · 搜索/崩溃日志              │  │ (仅增量)  │  │ · 周期性模式识别        │  │
│  │ · 过去30天全部可触达         │  │          │  └─────────────────────────┘  │
│  └──────────────────────────────┘  │          │                              │
│                                     │          │  ┌─ 行为预测 Agent ──────┐  │
│  ┌─ 本地 Tool（核心）────────────┐  │          │  │ · 读取 30 天滚动窗口    │  │
│  │ ① 扫描 30 天日志目录         │  │          │  │ · 加载用户画像          │  │
│  │ ② 数据清洗 + 去噪 + 聚合    │  │          │  │ · LLM 推理：            │  │
│  │ ③ 结构化 DSL 事件            │  │          │  │   短期意图(续编/模板)    │  │
│  │ ④ 上下文字段填充             │  │          │  │   周期性模式(月报/周报)  │  │
│  │ ⑤ 画像缓存附加               │  │          │  │ · 生成焦点项（≤5 条）   │  │
│  │ ⑥ 增量打包上传               │  │          │  │ · 输出结构化焦点 DSL    │  │
│  └──────────────────────────────┘  │          │  └─────────────────────────┘  │
│                                     │          │                              │
│  ┌─ 焦点渲染 ──────────────────┐  │  每日拉取  │  ┌─ 焦点项存储 + 下发 ───┐  │
│  │ · 首页焦点卡片列表           │  │←──────── │  │ · 存储到用户维度        │  │
│  │ · 排序展示（按优先级）       │  │ 焦点DSL   │  │ · 客户端启动时拉取      │  │
│  │ · 点击执行 action            │  │          │  │ · 附带更新后画像        │  │
│  └──────────────────────────────┘  │          │  └─────────────────────────┘  │
│                                     │          │                              │
│  ┌─ 反馈采集 ──────────────────┐  │          │  ┌─ 反馈回收 ─────────────┐  │
│  │ · 卡片点击（accepted）       │──反馈上报→│  │ · 焦点项点击/忽略统计   │  │
│  │ · 卡片关闭（dismissed）      │  │          │  │ · 纳入画像调整信号      │  │
│  │ · 无操作（ignored）          │  │          │  │ · 周期准确性验证        │  │
│  └──────────────────────────────┘  │          │  └─────────────────────────┘  │
└────────────────────────────────────┘          └──────────────────────────────┘
```"#;

    const HORIZONTAL_ARROW_DIAGRAM: &str = r#"```
┌────────────┐
│ ────────→  │
│ ←────────  │
│ DSL → add  │
└────────────┘
```"#;

    const WPS_ROLLING_WINDOW_DIAGRAM: &str = r#"```
时间轴（30天滚动窗口）

Day -29          Day -7           Day -1    Today    Day +1
  │                │                │         │         │
  ├────────────────┼────────────────┼─────────┼─────────┤
  │                 │                │         │         │
  │  30天历史行为   │  近期模式      │ 昨日    │ 今日    │ 次日展示
  │  (服务端存储)   │  (7天细粒度)   │ 增量    │ WPS     │ 焦点项
  │                 │                │ 上传    │ 首页    │
  │                 │                │         │         │
  ▼                 ▼                ▼         ▼         ▼
 ┌─────────────────────────────────────────────────────────┐
 │  服务端滚动窗口（始终保留最近 30 天行为摘要）             │
 │                                                         │
 │  每天凌晨：                                              │
 │    ① 接收昨日增量 DSL 包 → 追加到滚动窗口               │
 │    ② 淘汰 Day-31 旧数据                                 │
 │    ③ 更新画像（5个维度全量重算）                        │
 │    ④ 短期意图预测（基于 1-7 天近期行为）                │
 │    ⑤ 周期性模式检测（基于 7-30 天中长周期）             │
 │    ⑥ 合并焦点项 → 排序 → 存储                          │
 └─────────────────────────────────────────────────────────┘
```"#;

    fn build_and_render(md: &str) -> DrawList {
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let doc_view = core::document::StringDocView::new(md);
        let layout =
            layout_doc_with_shaper_for_rendering(&doc.blocks, &style, 400.0, None, None, &doc_view);
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
        render_doc_with_offset_and_ascii_diagrams(
            &layout.doc,
            &style,
            &mut dl,
            0.0,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
            Some(&layout.ascii_diagrams),
        );
        dl
    }

    fn build_and_render_editing(md: &str, cursor_byte: usize) -> (DrawList, MarkdownStyle) {
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let doc_view = core::document::StringDocView::new(md);
        let mut layout = LazyLayout::from_doc(doc, &style, 400.0, &doc_view);
        layout.set_edit_source(Some(md.to_string()));
        layout.set_edit_ctx(Some(crate::edit::EditContext {
            cursor_byte,
            preedit_text: None,
            preedit_cursor: None,
        }));

        let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
        layout.ensure_precise_range(0.0, 600.0, &style, &mut shaper, None, &doc_view);
        layout.build_flat_lines(&doc_view);

        let mut dl = DrawList::new();
        let laid_out = LaidOutDoc {
            blocks: layout.laid_out.iter().flatten().cloned().collect(),
            total_height: layout.total_height,
        };
        render_doc_with_offset_and_ascii_diagrams(
            &laid_out,
            &style,
            &mut dl,
            0.0,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
            Some(layout.ascii_diagrams()),
        );
        (dl, style)
    }

    fn build_and_render_with_highlighter(md: &str, highlighter: &dyn CodeHighlighter) -> DrawList {
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let doc_view = core::document::StringDocView::new(md);
        let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
        let layout = layout_doc_with_shaper_for_rendering(
            &doc.blocks,
            &style,
            400.0,
            Some(&mut shaper),
            Some(highlighter),
            &doc_view,
        );
        let mut dl = DrawList::new();
        render_doc_with_offset_and_ascii_diagrams(
            &layout.doc,
            &style,
            &mut dl,
            0.0,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
            Some(&layout.ascii_diagrams),
        );
        dl
    }

    fn build_laid_out(md: &str) -> MarkdownLayout {
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let doc_view = core::document::StringDocView::new(md);
        layout_doc_with_shaper_for_rendering(&doc.blocks, &style, 400.0, None, None, &doc_view)
    }

    fn render_laid_out(layout: &MarkdownLayout, viewport_height: f32) -> DrawList {
        let style = default_style();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
        render_doc_with_offset_and_ascii_diagrams(
            &layout.doc,
            &style,
            &mut draw_list,
            0.0,
            viewport_height,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
            Some(&layout.ascii_diagrams),
        );
        draw_list
    }

    fn line_text_xs(draw_list: &DrawList, line: &LaidOutLine, text: &str) -> Vec<f32> {
        let expected_baseline = line.rect.y + line.font_size;
        draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::TextLayout { layout, x, y_baseline, .. }
                    if layout.text == text && (*y_baseline - expected_baseline).abs() < 0.01 =>
                {
                    Some(*x)
                }
                _ => None,
            })
            .collect()
    }

    fn line_text_center_xs(draw_list: &DrawList, line: &LaidOutLine, text: &str) -> Vec<f32> {
        let expected_baseline = line.rect.y + line.font_size;
        draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::TextLayout { layout, x, y_baseline, .. }
                    if layout.text == text && (*y_baseline - expected_baseline).abs() < 0.01 =>
                {
                    Some(*x + layout.shaped.width * 0.5)
                }
                _ => None,
            })
            .collect()
    }

    fn fill_triangles_for_line(draw_list: &DrawList, line: &LaidOutLine) -> Vec<[[f32; 2]; 3]> {
        draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::FillTriangle { p0, p1, p2, .. }
                    if [p0, p1, p2].into_iter().all(|point| {
                        line.rect.y <= point[1] && point[1] <= line.rect.y + line.rect.h
                    }) =>
                {
                    Some([*p0, *p1, *p2])
                }
                _ => None,
            })
            .collect()
    }

    fn line_has_horizontal_fill_containing_x(
        draw_list: &DrawList,
        line: &LaidOutLine,
        target_x: f32,
    ) -> bool {
        const GEOMETRY_EPSILON: f32 = 0.01;

        let line_center_y = line.rect.y + line.rect.h * 0.5;
        draw_list.cmds.iter().any(|command| match command {
            DrawCmd::FillRect { rect, .. } => {
                let fill_center_y = rect.y + rect.h * 0.5;
                rect.w > rect.h
                    && rect.x <= target_x
                    && target_x <= rect.x + rect.w
                    && (fill_center_y - line_center_y).abs() < GEOMETRY_EPSILON
            }
            _ => false,
        })
    }

    fn assert_contains_x(actual_xs: &[f32], expected_x: f32, context: &str) {
        assert!(
            actual_xs.iter().any(|actual_x| (*actual_x - expected_x).abs() < 0.01),
            "{context} must contain x={expected_x}; actual={actual_xs:?}"
        );
    }

    fn text_command_color(draw_list: &DrawList, text: &str) -> Option<[f32; 4]> {
        draw_list.cmds.iter().find_map(|cmd| {
            if let DrawCmd::TextLayout { layout, color, .. } = cmd
                && layout.text == text
            {
                return Some(*color);
            }
            None
        })
    }

    fn text_command_colors(draw_list: &DrawList, text: &str) -> Vec<[f32; 4]> {
        draw_list
            .cmds
            .iter()
            .filter_map(|cmd| {
                if let DrawCmd::TextLayout { layout, color, .. } = cmd
                    && layout.text == text
                {
                    return Some(*color);
                }
                None
            })
            .collect()
    }

    #[test]
    fn render_paragraph_emits_text() {
        let dl = build_and_render("hello world");
        let has_text = dl.cmds.iter().any(|c| matches!(c, DrawCmd::TextLayout { .. }));
        assert!(has_text, "paragraph should produce Text commands");
    }

    #[test]
    fn render_heading_emits_text() {
        let dl = build_and_render("# Title");
        let has_text = dl.cmds.iter().any(|c| matches!(c, DrawCmd::TextLayout { .. }));
        assert!(has_text, "heading should produce Text commands");
    }

    #[test]
    fn render_code_block_emits_fill_rect() {
        let dl = build_and_render("```\ncode\n```");
        let has_fill = dl.cmds.iter().any(|c| matches!(c, DrawCmd::FillRect { .. }));
        assert!(has_fill, "code block should have background FillRect");
    }

    #[test]
    fn render_ascii_diagram_places_vertical_borders_on_one_grid_column() {
        let layout = build_laid_out("```\n┌────┐\n│中文│\n│内容│\n└────┘\n```");
        let code_block = layout.doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        let first_content_line =
            lines.iter().find(|line| line.text == "│中文│").expect("first content line");
        let second_content_line =
            lines.iter().find(|line| line.text == "│内容│").expect("second content line");

        let style = default_style();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
        render_doc_with_offset_and_ascii_diagrams(
            &layout.doc,
            &style,
            &mut draw_list,
            0.0,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
            Some(&layout.ascii_diagrams),
        );

        let first_borders = vertical_border_xs_for_line(&draw_list, first_content_line);
        let second_borders = vertical_border_xs_for_line(&draw_list, second_content_line);
        assert_eq!(
            first_borders.len(),
            2,
            "each content line must have left and right vertical borders"
        );
        assert_eq!(
            second_borders.len(),
            2,
            "each content line must have left and right vertical borders"
        );
        assert_eq!(
            first_borders, second_borders,
            "different content rows must share both border columns"
        );
    }

    #[test]
    fn render_snapped_rectangle_uses_one_right_edge_x() {
        let layout = build_laid_out(
            "```\n┌─ 本地日志（30天滚动）─────────┐\n│ · 文件操作（打开/关闭/保存）  │\n│ · 模板使用记录               │\n└──────────────────────────────┘\n```",
        );
        let code_block = layout.doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };

        let style = default_style();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
        render_doc_with_offset_and_ascii_diagrams(
            &layout.doc,
            &style,
            &mut draw_list,
            0.0,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
            Some(&layout.ascii_diagrams),
        );

        let right_edge_xs = lines
            .iter()
            .map(|line| {
                vertical_border_xs_for_line(&draw_list, line)
                    .last()
                    .copied()
                    .expect("every fixture row must draw a right edge")
            })
            .collect::<Vec<_>>();
        assert!(
            right_edge_xs.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 0.01),
            "all right-edge segments must share one x coordinate: {right_edge_xs:?}"
        );
    }

    #[test]
    fn render_electron_go_architecture_keeps_outer_tracks_and_arrow_separate() {
        const LEFT_FRAME_START_ROW: usize = 1;
        const LEFT_FRAME_END_ROW: usize = 11;
        const EXPECTED_LEFT_RIGHT_TRACK_INDEX: usize = 1;
        const EXPECTED_SERVER_LEFT_TRACK_INDEX: usize = 2;
        const EXPECTED_SERVER_RIGHT_TRACK_INDEX: usize = 3;

        let layout = build_laid_out(ELECTRON_GO_ARCHITECTURE_DIAGRAM);
        let code_block = layout.doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        assert_eq!(lines.len(), 19, "fixture must retain every architecture row");

        let draw_list = render_laid_out(&layout, 2_000.0);
        let top_border_xs = vertical_border_center_xs_for_line(&draw_list, &lines[1]);
        assert_eq!(top_border_xs.len(), 4, "top row must expose both outer rectangles");
        let left_right_x = top_border_xs[EXPECTED_LEFT_RIGHT_TRACK_INDEX];
        let server_left_x = top_border_xs[EXPECTED_SERVER_LEFT_TRACK_INDEX];
        let server_right_x = top_border_xs[EXPECTED_SERVER_RIGHT_TRACK_INDEX];

        for (frame_row, line) in lines[LEFT_FRAME_START_ROW..=LEFT_FRAME_END_ROW].iter().enumerate()
        {
            let border_xs = vertical_border_center_xs_for_line(&draw_list, line);
            assert_contains_x(
                &border_xs,
                left_right_x,
                &format!("left outer right track row {frame_row}"),
            );
        }

        for (row_index, line) in lines[1..].iter().enumerate() {
            let border_xs = vertical_border_center_xs_for_line(&draw_list, line);
            assert_contains_x(
                &border_xs,
                server_left_x,
                &format!("server left track row {row_index}"),
            );
            assert_contains_x(
                &border_xs,
                server_right_x,
                &format!("server right track row {row_index}"),
            );
        }

        let arrow_centers = line_text_center_xs(&draw_list, &lines[3], "►");
        assert_eq!(arrow_centers.len(), 1, "connector row must draw one right arrow head");
        assert!(
            left_right_x < arrow_centers[0] && arrow_centers[0] < server_left_x,
            "arrow must stay between the frames: left={left_right_x}, arrow={}, right={server_left_x}",
            arrow_centers[0]
        );
    }

    #[test]
    fn render_horizontal_connector_arrows_share_box_line_center() {
        const GEOMETRY_EPSILON: f32 = 0.01;

        let layout = build_laid_out(HORIZONTAL_ARROW_DIAGRAM);
        let code_block = layout.doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        let draw_list = render_laid_out(&layout, 600.0);

        let right_arrow = fill_triangles_for_line(&draw_list, &lines[1]);
        let left_arrow = fill_triangles_for_line(&draw_list, &lines[2]);
        assert_eq!(right_arrow.len(), 1, "right connector must draw one arrowhead");
        assert_eq!(left_arrow.len(), 1, "left connector must draw one arrowhead");

        for (line, triangle) in [(&lines[1], right_arrow[0]), (&lines[2], left_arrow[0])] {
            let minimum_y = triangle.iter().map(|point| point[1]).fold(f32::INFINITY, f32::min);
            let maximum_y = triangle.iter().map(|point| point[1]).fold(f32::NEG_INFINITY, f32::max);
            let arrow_center_y = (minimum_y + maximum_y) * 0.5;
            let box_line_center_y = line.rect.y + line.rect.h * 0.5;
            assert!((arrow_center_y - box_line_center_y).abs() < GEOMETRY_EPSILON);
        }

        assert!(right_arrow[0][0][0] > right_arrow[0][1][0]);
        assert!(right_arrow[0][0][0] > right_arrow[0][2][0]);
        assert!(left_arrow[0][0][0] < left_arrow[0][1][0]);
        assert!(left_arrow[0][0][0] < left_arrow[0][2][0]);
        assert!(line_has_horizontal_fill_containing_x(&draw_list, &lines[1], right_arrow[0][1][0]));
        assert!(line_has_horizontal_fill_containing_x(&draw_list, &lines[2], left_arrow[0][1][0]));
        assert!(line_text_xs(&draw_list, &lines[1], "→").is_empty());
        assert!(line_text_xs(&draw_list, &lines[2], "←").is_empty());
        assert!(fill_triangles_for_line(&draw_list, &lines[3]).is_empty());
        assert_eq!(line_text_xs(&draw_list, &lines[3], "→").len(), 1);
    }

    #[test]
    fn render_wps_ring_buffer_uses_one_right_edge_x_with_discontinuous_source_columns() {
        let layout = build_laid_out(
            "```\n每个用户维护一个环形缓冲区：\n\n┌──────────────────────────────────────────────────────────┐\n│  Day-30  Day-29  ...  Day-2  Day-1  Today  ← 窗口滑动    │\n│  ─────  ─────        ─────  ─────  ─────                 │\n│  淘汰    保留         保留   保留   新增                  │\n│                                                          │\n│  每天凌晨批处理时：                                       │\n│    1. 追加昨日增量包到窗口尾部                            │\n│    2. 如果窗口长度 > 30，淘汰最早的 Day-31               │\n│    3. 基于完整 30 天窗口重算画像 + 检测周期               │\n└──────────────────────────────────────────────────────────┘\n```",
        );
        let code_block = layout.doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };

        let style = default_style();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
        render_doc_with_offset_and_ascii_diagrams(
            &layout.doc,
            &style,
            &mut draw_list,
            0.0,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
            Some(&layout.ascii_diagrams),
        );

        let right_edge_xs = lines
            .iter()
            .filter(|line| line.text.starts_with(['┌', '│', '└']))
            .map(|line| {
                vertical_border_xs_for_line(&draw_list, line)
                    .last()
                    .copied()
                    .expect("every outer-frame row must draw a right edge")
            })
            .collect::<Vec<_>>();
        assert!(
            right_edge_xs.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 0.01),
            "all existing right-edge segments must share one x coordinate: {right_edge_xs:?}"
        );
    }

    #[test]
    fn render_wps_architecture_accumulates_all_outer_tracks_from_the_left_edge() {
        const EXPECTED_OUTER_COLUMNS: [usize; 4] = [0, 38, 51, 84];

        let layout = build_laid_out(WPS_ARCHITECTURE_DIAGRAM);
        let code_block = layout.doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        assert_eq!(lines.len(), 31, "fixture must retain every architecture row");

        let style = default_style();
        let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
        let cell_width =
            code_cell_width(&mut shaper, style.code_font_size, style.code_font_family.as_deref());
        let draw_list = render_laid_out(&layout, 2_000.0);

        let top_border_xs = vertical_border_xs_for_line(&draw_list, &lines[0]);
        assert_eq!(top_border_xs.len(), EXPECTED_OUTER_COLUMNS.len());
        let client_left_x = top_border_xs[0];
        let expected_outer_xs =
            EXPECTED_OUTER_COLUMNS.map(|column| client_left_x + column as f32 * cell_width);
        for (actual_x, expected_x) in top_border_xs.iter().zip(expected_outer_xs) {
            assert!(
                (*actual_x - expected_x).abs() < 0.01,
                "top-level track must use cumulative column spacing: actual={top_border_xs:?}, expected={expected_outer_xs:?}"
            );
        }

        for (row_index, line) in lines.iter().enumerate() {
            let row_border_xs = vertical_border_xs_for_line(&draw_list, line);
            let expected_track_indices: &[usize] =
                if row_index == 26 { &[0, 2, 3] } else { &[0, 1, 2, 3] };
            for track_index in expected_track_indices.iter().copied() {
                assert_contains_x(
                    &row_border_xs,
                    expected_outer_xs[track_index],
                    &format!("architecture row {row_index}, outer track {track_index}"),
                );
            }
        }

        let feedback_line = &lines[26];
        let feedback_border_xs = vertical_border_xs_for_line(&draw_list, feedback_line);
        assert!(
            !feedback_border_xs
                .iter()
                .any(|actual_x| (*actual_x - expected_outer_xs[1]).abs() < 0.01),
            "feedback row must preserve the missing client-right outer edge"
        );
        let feedback_arrow_xs = line_text_xs(&draw_list, feedback_line, "→");
        assert_eq!(feedback_arrow_xs.len(), 1, "feedback row must draw exactly one arrow");
        assert!(
            expected_outer_xs[1] < feedback_arrow_xs[0]
                && feedback_arrow_xs[0] < expected_outer_xs[2],
            "feedback arrow must inherit the prefix shift and stay between client and server: arrow={}, client_right={}, server_left={}",
            feedback_arrow_xs[0],
            expected_outer_xs[1],
            expected_outer_xs[2]
        );
    }

    #[test]
    fn render_wps_rolling_window_includes_bottom_corners_in_both_outer_tracks() {
        const FRAME_START_ROW: usize = 11;
        const FRAME_END_ROW: usize = 21;

        let layout = build_laid_out(WPS_ROLLING_WINDOW_DIAGRAM);
        let code_block = layout.doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        assert_eq!(lines.len(), FRAME_END_ROW + 1, "fixture must retain the bottom border row");

        let draw_list = render_laid_out(&layout, 2_000.0);
        let frame_border_xs = lines[FRAME_START_ROW..=FRAME_END_ROW]
            .iter()
            .enumerate()
            .map(|(frame_row, line)| {
                let border_xs = vertical_border_xs_for_line(&draw_list, line);
                assert_eq!(
                    border_xs.len(),
                    2,
                    "rolling-window frame row {} must draw both outer borders",
                    FRAME_START_ROW + frame_row
                );
                [border_xs[0], border_xs[1]]
            })
            .collect::<Vec<_>>();
        let expected_left_x = frame_border_xs[0][0];
        let expected_right_x = frame_border_xs[0][1];
        for (frame_row, [left_x, right_x]) in frame_border_xs.iter().copied().enumerate() {
            assert!(
                (left_x - expected_left_x).abs() < 0.01
                    && (right_x - expected_right_x).abs() < 0.01,
                "rolling-window row {} must share both outer x coordinates: expected=[{}, {}], actual=[{}, {}]",
                FRAME_START_ROW + frame_row,
                expected_left_x,
                expected_right_x,
                left_x,
                right_x
            );
        }
    }

    #[test]
    fn render_wps_open_timeline_uses_shared_vertical_track_xs() {
        const FIRST_TRACK_ROW: usize = 3;
        const LAST_TRACK_ROW: usize = 9;
        const TRACK_COUNT: usize = 5;

        let layout = build_laid_out(WPS_ROLLING_WINDOW_DIAGRAM);
        let code_block = layout.doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        let draw_list = render_laid_out(&layout, 2_000.0);
        let expected_xs = vertical_border_xs_for_line(&draw_list, &lines[FIRST_TRACK_ROW]);
        assert_eq!(expected_xs.len(), TRACK_COUNT, "fixture must expose five timeline tracks");

        for (row_index, line) in
            lines.iter().enumerate().take(LAST_TRACK_ROW + 1).skip(FIRST_TRACK_ROW + 1)
        {
            let actual_xs = vertical_border_xs_for_line(&draw_list, line);
            assert_eq!(
                actual_xs, expected_xs,
                "timeline row {row_index} must share all vertical track x coordinates"
            );
        }

        let stem_center_xs = vertical_border_center_xs_for_line(&draw_list, &lines[LAST_TRACK_ROW]);
        let arrow_line = &lines[LAST_TRACK_ROW + 1];
        let arrow_center_xs = line_text_center_xs(&draw_list, arrow_line, "▼");
        assert_eq!(arrow_center_xs.len(), TRACK_COUNT, "all arrowheads must remain text cells");
        assert!(
            stem_center_xs
                .iter()
                .zip(&arrow_center_xs)
                .all(|(stem_x, arrow_x)| (*stem_x - *arrow_x).abs() < 0.01),
            "arrowheads must share the vertical track centers: stems={stem_center_xs:?}, arrows={arrow_center_xs:?}"
        );
        assert!(
            vertical_border_xs_for_line(&draw_list, arrow_line).is_empty(),
            "arrowheads must not be converted into box-line geometry"
        );
    }

    #[test]
    fn render_snapped_rectangle_extends_a_shifted_corner_connection() {
        let layout = build_laid_out("```\n┌────┐\n│x │\n└───┘\n```");
        let code_block = layout.doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        let bottom_line = lines.iter().find(|line| line.text == "└───┘").expect("bottom line");
        let diagram = layout
            .ascii_diagrams
            .diagram_for(lines)
            .expect("fixture must retain its ASCII diagram sidecar");
        let shifted_corner = diagram.rows[2]
            .cells
            .iter()
            .find(|cell| cell.text == "┘")
            .expect("bottom row must contain the shifted right corner");
        let source_column = shifted_corner.column;
        let render_column = shifted_corner.render_column();
        assert!(render_column > source_column, "fixture must shift the bottom right corner");

        let style = default_style();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
        let cell_width =
            code_cell_width(&mut shaper, style.code_font_size, style.code_font_family.as_deref());
        render_doc_with_offset_and_ascii_diagrams(
            &layout.doc,
            &style,
            &mut draw_list,
            0.0,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
            Some(&layout.ascii_diagrams),
        );

        let thickness = (bottom_line.font_size * 0.08).clamp(1.0, 2.0);
        let half_thickness = thickness * 0.5;
        let corner_x = bottom_line.rect.x + render_column as f32 * cell_width;
        let extension_width = (render_column - source_column) as f32 * cell_width;
        let expected_horizontal_x = corner_x - extension_width;
        let expected_horizontal_width = extension_width + cell_width * 0.5 + half_thickness;
        let expected_horizontal_y = bottom_line.rect.y + bottom_line.rect.h * 0.5 - half_thickness;
        let expected_vertical_x = corner_x + cell_width * 0.5 - half_thickness;
        let expected_vertical_height = bottom_line.rect.h * 0.5 + half_thickness;
        const GEOMETRY_EPSILON: f32 = 0.01;

        let horizontal_connection = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::FillRect { rect, .. }
                    if (rect.x - expected_horizontal_x).abs() < GEOMETRY_EPSILON
                        && (rect.y - expected_horizontal_y).abs() < GEOMETRY_EPSILON
                        && (rect.w - expected_horizontal_width).abs() < GEOMETRY_EPSILON
                        && (rect.h - thickness).abs() < GEOMETRY_EPSILON =>
                {
                    Some(rect)
                }
                _ => None,
            })
            .expect("shifted corner must draw its exact extended left connection");
        let right_vertical_connection = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::FillRect { rect, .. }
                    if (rect.x - expected_vertical_x).abs() < GEOMETRY_EPSILON
                        && (rect.y - bottom_line.rect.y).abs() < GEOMETRY_EPSILON
                        && (rect.w - thickness).abs() < GEOMETRY_EPSILON
                        && (rect.h - expected_vertical_height).abs() < GEOMETRY_EPSILON =>
                {
                    Some(rect)
                }
                _ => None,
            })
            .expect("shifted corner must draw its right vertical connection");

        assert!(
            ((horizontal_connection.x + horizontal_connection.w)
                - (right_vertical_connection.x + right_vertical_connection.w))
                .abs()
                < GEOMETRY_EPSILON,
            "extended horizontal connection must end with the expected overlap on the right vertical edge"
        );
    }

    #[test]
    fn render_keeps_shifted_text_beyond_a_corner_extension() {
        let layout = build_laid_out("```\n┌──┐ X\n│  │\n└────┘\n```");
        let code_block = layout.doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        let top_line = lines.iter().find(|line| line.text == "┌──┐ X").expect("top line");
        let diagram = layout
            .ascii_diagrams
            .diagram_for(lines)
            .expect("fixture must retain its ASCII diagram sidecar");
        let text_cell = diagram.rows[0]
            .cells
            .iter()
            .find(|cell| cell.text == "X")
            .expect("top row must retain the shifted text cell");

        let style = default_style();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
        let cell_width =
            code_cell_width(&mut shaper, style.code_font_size, style.code_font_family.as_deref());
        render_doc_with_offset_and_ascii_diagrams(
            &layout.doc,
            &style,
            &mut draw_list,
            0.0,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
            Some(&layout.ascii_diagrams),
        );

        assert_eq!(text_cell.render_column(), 7);
        let text_center_x = top_line.rect.x
            + (text_cell.render_column() as f32 + text_cell.column_width as f32 * 0.5) * cell_width;
        let extension_overlaps_shifted_text = draw_list.cmds.iter().any(|command| match command {
            DrawCmd::FillRect { rect, .. } => {
                rect.y >= top_line.rect.y
                    && rect.y + rect.h <= top_line.rect.y + top_line.rect.h
                    && rect.h <= 2.0
                    && rect.x < text_center_x
                    && text_center_x < rect.x + rect.w
            }
            _ => false,
        });
        assert!(
            !extension_overlaps_shifted_text,
            "the corner extension must end before text shifted with the row suffix"
        );
    }

    #[test]
    fn render_keeps_shifted_text_beyond_a_moved_middle_edge() {
        let layout = build_laid_out("```\n┌────┐\n│  │ X\n└────┘\n```");
        let code_block = layout.doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &code_block.kind else {
            panic!("fixture must produce a code block");
        };
        let middle_line = lines.iter().find(|line| line.text == "│  │ X").expect("middle line");
        let diagram = layout
            .ascii_diagrams
            .diagram_for(lines)
            .expect("fixture must retain its ASCII diagram sidecar");
        let text_cell = diagram.rows[1]
            .cells
            .iter()
            .find(|cell| cell.text == "X")
            .expect("middle row must retain the shifted text cell");

        let style = default_style();
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
        let cell_width =
            code_cell_width(&mut shaper, style.code_font_size, style.code_font_family.as_deref());
        render_doc_with_offset_and_ascii_diagrams(
            &layout.doc,
            &style,
            &mut draw_list,
            0.0,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
            Some(&layout.ascii_diagrams),
        );

        assert_eq!(text_cell.render_column(), 7);
        let text_center_x = middle_line.rect.x
            + (text_cell.render_column() as f32 + text_cell.column_width as f32 * 0.5) * cell_width;
        let vertical_edge_overlaps_text = draw_list.cmds.iter().any(|command| match command {
            DrawCmd::FillRect { rect, .. } => {
                rect.y >= middle_line.rect.y
                    && rect.y + rect.h <= middle_line.rect.y + middle_line.rect.h
                    && rect.w <= 2.0
                    && rect.h > 8.0
                    && rect.x <= text_center_x
                    && text_center_x <= rect.x + rect.w
            }
            _ => false,
        });
        assert!(
            !vertical_edge_overlaps_text,
            "the middle edge must remain left of text shifted with the row suffix"
        );
    }

    fn vertical_border_xs_for_line(draw_list: &DrawList, line: &LaidOutLine) -> Vec<f32> {
        let line_bottom = line.rect.y + line.rect.h;
        let mut xs = draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::FillRect { rect, .. }
                    if rect.w <= 2.0
                        && rect.h > 8.0
                        && rect.y >= line.rect.y
                        && rect.y + rect.h <= line_bottom =>
                {
                    Some(rect.x)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        xs.sort_by(f32::total_cmp);
        xs.dedup_by(|left, right| (*left - *right).abs() < 0.01);
        xs
    }

    fn vertical_border_center_xs_for_line(draw_list: &DrawList, line: &LaidOutLine) -> Vec<f32> {
        let line_bottom = line.rect.y + line.rect.h;
        let mut xs = draw_list
            .cmds
            .iter()
            .filter_map(|command| match command {
                DrawCmd::FillRect { rect, .. }
                    if rect.w <= 2.0
                        && rect.h > 8.0
                        && rect.y >= line.rect.y
                        && rect.y + rect.h <= line_bottom =>
                {
                    Some(rect.x + rect.w * 0.5)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        xs.sort_by(f32::total_cmp);
        xs.dedup_by(|left, right| (*left - *right).abs() < 0.01);
        xs
    }

    #[test]
    fn render_normal_code_block_keeps_single_text_line_path() {
        let dl = build_and_render("```\nlet value = 1;\n```");
        let text_count = dl
            .cmds
            .iter()
            .filter(|command| {
                matches!(command, DrawCmd::TextLayout { layout, .. } if layout.text == "let value = 1;")
            })
            .count();
        assert_eq!(text_count, 1, "normal code must not be split into grid cells");
    }

    #[test]
    fn render_active_ascii_diagram_keeps_text_path() {
        let source = "```\n┌────┐\n│中文│\n└────┘\n```";
        let cursor = source.find("中文").expect("fixture has CJK label");
        let (dl, _) = build_and_render_editing(source, cursor);
        assert!(dl.cmds.iter().any(
            |command| matches!(command, DrawCmd::TextLayout { layout, .. } if layout.text.contains("中文"))
        ));
    }

    #[test]
    fn render_ascii_diagram_preserves_highlight_span_colors() {
        const HIGHLIGHT_COLOR: [f32; 4] = [0.9, 0.3, 0.1, 1.0];

        struct DiagramHighlighter;

        impl CodeHighlighter for DiagramHighlighter {
            fn highlight(&self, _language: &str, code: &str) -> Vec<Vec<HighlightSpan>> {
                code.lines()
                    .map(|line| {
                        if line == "│fn │" {
                            vec![HighlightSpan {
                                start: "│".len(),
                                len: "fn".len(),
                                color: HIGHLIGHT_COLOR,
                            }]
                        } else {
                            vec![]
                        }
                    })
                    .collect()
            }
        }

        let dl = build_and_render_with_highlighter(
            "```diagram\n┌───┐\n│fn │\n└───┘\n```",
            &DiagramHighlighter,
        );
        let highlighted_cells = dl
            .cmds
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    DrawCmd::TextLayout { layout, color, .. }
                        if (layout.text == "f" || layout.text == "n") && *color == HIGHLIGHT_COLOR
                )
            })
            .count();

        assert_eq!(highlighted_cells, 2, "grid cells must retain their syntax highlight color");
    }

    #[test]
    fn render_ascii_diagram_without_shaper_skips_grid_geometry() {
        let layout = build_laid_out("```\n┌────┐\n│中文│\n└────┘\n```");
        let style = default_style();
        let mut dl = DrawList::new();
        render_doc_with_offset_and_ascii_diagrams(
            &layout.doc,
            &style,
            &mut dl,
            0.0,
            600.0,
            0.0,
            0.0,
            None,
            &[],
            Some(&layout.ascii_diagrams),
        );

        assert!(
            !dl.cmds.iter().any(|command| {
                matches!(command, DrawCmd::FillRect { rect, .. } if rect.w <= 2.0 && rect.h > 8.0)
            }),
            "without a shaper the renderer must keep the existing non-grid fallback"
        );
    }

    #[test]
    fn render_ascii_diagram_unknown_box_drawing_character_uses_text_fallback() {
        let draw_list = build_and_render("```\n┌──┐\n│═ │\n└──┘\n```");
        assert!(draw_list.cmds.iter().any(
            |command| matches!(command, DrawCmd::TextLayout { layout, .. } if layout.text == "═")
        ));
    }

    #[test]
    fn render_ascii_diagram_with_mismatched_rows_keeps_text_line_path() {
        let mut layout = build_laid_out("```\n┌────┐\n│中文│\n└────┘\n```");
        let (doc, ascii_diagrams) = (&mut layout.doc, &mut layout.ascii_diagrams);
        let block = doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
            panic!("fixture must produce a code block");
        };
        ascii_diagrams.remove_last_row_for(lines);

        let style = default_style();
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("need shaper for render tests");
        render_doc_with_offset_and_ascii_diagrams(
            doc,
            &style,
            &mut dl,
            0.0,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
            Some(ascii_diagrams),
        );

        assert!(dl.cmds.iter().any(
            |command| matches!(command, DrawCmd::TextLayout { layout, .. } if layout.text == "│中文│")
        ));
    }

    #[test]
    fn render_horizontal_rule_emits_fill() {
        let dl = build_and_render("text\n\n---");
        let has_fill = dl.cmds.iter().any(|c| matches!(c, DrawCmd::FillRect { .. }));
        assert!(has_fill, "horizontal rule should emit FillRect");
    }

    #[test]
    fn render_empty_doc_is_noop() {
        let dl = build_and_render("");
        // Empty doc: only PushClip + PopClip
        assert_eq!(dl.cmds.len(), 2, "empty doc should only have clip commands");
    }

    #[test]
    fn render_inline_code_has_background() {
        let dl = build_and_render("use `println!` here");
        // Should have at least one FillRect (inline code background)
        let fill_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillRect { .. })).count();
        assert!(fill_count >= 1, "inline code should have background FillRect, got {}", fill_count);
        // Should have text commands for the segments
        let text_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::TextLayout { .. })).count();
        assert!(
            text_count >= 2,
            "should have multiple text segments for styled line, got {}",
            text_count
        );
    }

    #[test]
    fn render_inline_code_background_does_not_overlap_adjacent_text() {
        const BACKGROUND_EDGE_TOLERANCE_PX: f32 = 0.5;

        let dl = build_and_render("a`code`b");
        let code_text = dl
            .cmds
            .iter()
            .find_map(|cmd| {
                if let DrawCmd::TextLayout { layout, x, .. } = cmd
                    && layout.text == "code"
                {
                    return Some((*x, layout.shaped.width));
                }
                None
            })
            .expect("inline code text should render as its own text command");
        let background_rect = dl
            .cmds
            .iter()
            .find_map(|cmd| {
                if let DrawCmd::FillRect { rect, color, .. } = cmd
                    && *color == default_style().inline_code_bg
                {
                    return Some(*rect);
                }
                None
            })
            .expect("inline code background should render");

        let code_left = code_text.0;
        let code_right = code_text.0 + code_text.1;
        let background_right = background_rect.x + background_rect.w;

        assert!(
            background_rect.x + BACKGROUND_EDGE_TOLERANCE_PX >= code_left,
            "inline code background should not enter preceding text; background={background_rect:?}, code_left={code_left}"
        );
        assert!(
            background_right <= code_right + BACKGROUND_EDGE_TOLERANCE_PX,
            "inline code background should not enter following text; background={background_rect:?}, code_right={code_right}"
        );
    }

    #[test]
    fn render_inline_code_background_keeps_padding_when_spacing_allows() {
        let dl = build_and_render("a `code` b");
        let code_text = dl
            .cmds
            .iter()
            .find_map(|cmd| {
                if let DrawCmd::TextLayout { layout, x, .. } = cmd
                    && layout.text == "code"
                {
                    return Some((*x, layout.shaped.width));
                }
                None
            })
            .expect("inline code text should render as its own text command");
        let background_rect = dl
            .cmds
            .iter()
            .find_map(|cmd| {
                if let DrawCmd::FillRect { rect, color, .. } = cmd
                    && *color == default_style().inline_code_bg
                {
                    return Some(*rect);
                }
                None
            })
            .expect("inline code background should render");

        assert!(
            background_rect.x < code_text.0,
            "inline code background should add left padding when spacing allows; background={background_rect:?}, code_x={}",
            code_text.0
        );
        assert!(
            background_rect.x + background_rect.w > code_text.0 + code_text.1,
            "inline code background should add right padding when spacing allows; background={background_rect:?}, code_right={}",
            code_text.0 + code_text.1
        );
    }

    #[test]
    fn render_inline_code_color_differs_from_base() {
        let md = "`code`";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
        let mut dl = DrawList::new();
        let mut test_shaper = shaping::Shaper::new().unwrap();
        render_doc(&laid_out, &style, &mut dl, 0.0, 600.0, Some(&mut test_shaper));
        // Find the text command for inline code - should use code_color
        let code_texts: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let DrawCmd::TextLayout { layout, color, .. } = c {
                    if layout.text.contains("code") { Some(*color) } else { None }
                } else {
                    None
                }
            })
            .collect();
        assert!(!code_texts.is_empty(), "should render inline code text");
        // code_color should differ from link_color (sanity check)
        assert_ne!(code_texts[0], style.link_color, "inline code should not use link color");
    }

    #[test]
    fn render_strikethrough_thickness_scales_with_high_dpi_font_size() {
        let markdown = "~~deleted~~";
        let parsed = parse_markdown(markdown);
        let mut style = default_style();
        style.body_font_size *= 2.0;
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 800.0, &core::document::StringDocView::new(markdown));
        let mut draw_list = DrawList::new();
        let mut shaper = shaping::Shaper::new().expect("render test requires a text shaper");
        render_doc(&laid_out, &style, &mut draw_list, 0.0, 600.0, Some(&mut shaper));

        let strikethrough = draw_list
            .cmds
            .iter()
            .find_map(|command| match command {
                DrawCmd::FillRect { rect, color, .. }
                    if *color == style.text_color && rect.w > rect.h =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .expect("strikethrough should render as a text-colored rectangle");

        assert_eq!(strikethrough.h, 2.0, "2x DPI should produce a 2px strikethrough");
    }

    #[test]
    fn render_wysiwyg_source_marker_uses_muted_color() {
        let (draw_list, style) = build_and_render_editing("# Title", 3);
        let marker_color =
            text_command_color(&draw_list, "# ").expect("marker should render as its own text");
        let text_color =
            text_command_color(&draw_list, "Title").expect("body text should render separately");
        let expected_marker_color =
            blend_toward_bg(style.heading_color, style.background_color, SOURCE_MARKER_FADE_RATIO);

        assert_eq!(text_color, style.heading_color, "heading text should keep heading color");
        assert_eq!(marker_color, expected_marker_color, "source marker should be muted");
    }

    #[test]
    fn render_wysiwyg_inline_source_markers_use_muted_color() {
        let (draw_list, style) = build_and_render_editing("**bold**", 3);
        let marker_colors = text_command_colors(&draw_list, "**");
        let bold_color =
            text_command_color(&draw_list, "bold").expect("bold text should render separately");
        let expected_marker_color =
            blend_toward_bg(style.text_color, style.background_color, SOURCE_MARKER_FADE_RATIO);

        assert_eq!(marker_colors.len(), 2, "both bold markers should render separately");
        assert!(marker_colors.iter().all(|color| *color == expected_marker_color));
        assert_eq!(bold_color, style.text_color, "bold content should keep normal color");
    }

    #[test]
    fn render_table_has_zebra_stripes() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n| 5 | 6 |";
        let dl = build_and_render(md);
        // Should have multiple FillRect commands for header bg, stripe bg, separators
        let fill_count = dl.cmds.iter().filter(|c| matches!(c, DrawCmd::FillRect { .. })).count();
        // At least: header bg + header separator + stripe for row 2 + row separators + vertical grid
        assert!(
            fill_count >= 5,
            "table with 3 body rows should have stripe fills, got {}",
            fill_count
        );
    }

    #[test]
    fn render_blockquote_uses_bg_color() {
        let dl = build_and_render("> quote");
        // Find the blockquote background fill (rounded rect with blockquote_bg alpha)
        let fills: Vec<_> =
            dl.cmds
                .iter()
                .filter_map(|c| {
                    if let DrawCmd::FillRect { color, .. } = c { Some(*color) } else { None }
                })
                .collect();
        // blockquote_bg has low alpha (0.08 dark, 0.05 light)
        let has_low_alpha_bg = fills.iter().any(|c| c[3] < 0.15);
        assert!(
            has_low_alpha_bg,
            "blockquote should have low-alpha background fill, got {:?}",
            fills
        );
    }

    #[test]
    fn render_table_single_row_no_extra_stripe() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
        let mut dl = DrawList::new();
        render_doc(&laid_out, &style, &mut dl, 0.0, 600.0, None);
        // Single body row (row_idx=0, even) should NOT get stripe bg
        // The stripe bg uses table_stripe_bg which has alpha 1.0
        // Count fills with table_stripe_bg color
        let stripe_color = style.table_stripe_bg;
        let stripe_count = dl
            .cmds
            .iter()
            .filter(|c| matches!(c, DrawCmd::FillRect { color, .. } if *color == stripe_color))
            .count();
        assert_eq!(stripe_count, 0, "single body row (even index) should have no stripe");
    }

    #[test]
    fn render_bold_text_uses_bold_weight() {
        let dl = build_and_render("**bold** text");
        // Find text command for "bold"
        let bold_cmds: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let DrawCmd::TextLayout { layout, .. } = c {
                    if layout.text.contains("bold") { Some(layout.font_weight) } else { None }
                } else {
                    None
                }
            })
            .collect();
        assert!(
            bold_cmds.contains(&Weight::SEMIBOLD),
            "bold text should use Weight::SEMIBOLD, got {:?}",
            bold_cmds
        );
    }

    #[test]
    fn render_italic_text_uses_italic_flag() {
        let dl = build_and_render("*italic* text");
        let italic_cmds: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let DrawCmd::TextLayout { layout, .. } = c {
                    if layout.text.contains("italic") { Some(layout.italic) } else { None }
                } else {
                    None
                }
            })
            .collect();
        assert!(
            italic_cmds.iter().any(|&italic| italic),
            "italic text should have italic flag set, got {:?}",
            italic_cmds
        );
    }

    #[test]
    fn render_code_line_uses_monospace_family() {
        let dl = build_and_render("```\ncode\n```");
        // Code block text should use monospace font family
        let code_texts: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let DrawCmd::TextLayout { layout, .. } = c {
                    if layout.text.contains("code") {
                        Some(layout.font_family.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        assert!(
            code_texts.iter().any(|f| f.as_deref() == Some("monospace")),
            "code block should use monospace font, got {:?}",
            code_texts
        );
    }

    #[test]
    fn render_body_text_font_family_restored() {
        let dl = build_and_render("normal **bold** normal");
        // Collect all text commands with their font families
        let all_texts: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let DrawCmd::TextLayout { layout, .. } = c {
                    Some((layout.text.clone(), layout.font_family.clone()))
                } else {
                    None
                }
            })
            .collect();
        // All body text (non-code) should have sans-serif font family
        for (content, family) in &all_texts {
            if !content.is_empty() {
                assert_eq!(
                    family.as_deref(),
                    Some("PingFang SC"),
                    "body text '{}' should use PingFang SC font family, got {:?}",
                    content,
                    family
                );
            }
        }
    }

    #[test]
    fn nested_list_bullet_varies_by_depth() {
        // Top-level: "•", nested: "◦"
        let md = "- top\n  - nested";
        let dl = build_and_render(md);
        let bullets: Vec<String> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let DrawCmd::TextLayout { layout, .. } = c {
                    if layout.text == "•" || layout.text == "◦" || layout.text == "▪" {
                        Some(layout.text.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        assert!(bullets.contains(&"•".to_string()), "top-level should use •");
        assert!(bullets.contains(&"◦".to_string()), "nested should use ◦");
    }

    #[test]
    fn unchecked_tasklist_has_subtle_border() {
        let md = "- [ ] unchecked\n- [x] checked";
        let dl = build_and_render(md);
        // All checkbox borders should be fully opaque (blend_toward_bg keeps alpha=1.0)
        let strokes: Vec<[f32; 4]> =
            dl.cmds
                .iter()
                .filter_map(|c| {
                    if let DrawCmd::StrokeRect { color, .. } = c { Some(*color) } else { None }
                })
                .collect();
        assert!(
            strokes.iter().all(|c| c[3] >= 0.99),
            "all borders should be opaque, got {:?}",
            strokes
        );
        // Unchecked border should be blended toward background (dimmer than checked)
        let _bg = [1.0, 1.0, 1.0, 1.0]; // test theme bg
        let unchecked = strokes[0];
        let checked = strokes[1];
        let unchecked_lum = unchecked[0] + unchecked[1] + unchecked[2];
        let checked_lum = checked[0] + checked[1] + checked[2];
        // Unchecked is blended toward bg so in light theme it should be brighter (higher sum)
        assert!(unchecked_lum != checked_lum, "unchecked and checked should differ visually");
        // Checked state should render a x checkmark
        let has_checkmark = dl.cmds.iter().any(|c| {
            if let DrawCmd::TextLayout { layout, .. } = c { layout.text == "x" } else { false }
        });
        assert!(has_checkmark, "checked checkbox should render a x checkmark");
    }

    #[test]
    fn render_table_cell_text_no_chars_lost_at_different_widths() {
        // End-to-end: render table with CJK + inline code at multiple widths.
        // Extract all rendered text from DrawList and verify no characters lost.
        let md = "| 状图（├── `mod.rs  # comment`）整行 |\n|---|\n| x |";
        let source_texts = ["状图（├──", "mod.rs  # comment", "）整行"];
        for &w in &[100.0, 188.0, 300.0, 500.0, 800.0] {
            let parsed = parse_markdown(md);
            let style = default_style();
            let doc = MarkdownDoc::build(&parsed, &style);
            let laid_out =
                layout_doc(&doc.blocks, &style, w, &core::document::StringDocView::new(md));
            let mut dl = DrawList::new();
            let mut shaper = shaping::Shaper::new().expect("need shaper");
            render_doc(&laid_out, &style, &mut dl, 0.0, 600.0, Some(&mut shaper));
            // Collect all rendered text
            let mut rendered = String::new();
            for cmd in &dl.cmds {
                if let DrawCmd::TextLayout { layout, .. } = cmd {
                    rendered.push_str(&layout.text);
                }
            }
            for expected in &source_texts {
                assert!(
                    rendered.contains(expected),
                    "missing {:?} at w={}: rendered={:?}",
                    expected,
                    w,
                    rendered
                );
            }
        }
    }

    #[test]
    fn first_visible_block_idx_finds_block() {
        let md = "# H1\n\nparagraph text\n\n## H2";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
        assert!(laid_out.blocks.len() >= 2);

        // At top: first block visible
        assert_eq!(first_visible_block_idx(&laid_out.blocks, &[], 0.0), 0);

        // Scroll past first block
        let past_h1 = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h + 1.0;
        assert!(first_visible_block_idx(&laid_out.blocks, &[], past_h1) >= 1);

        // Scroll past end: clamp to last
        let way_past = laid_out.total_height + 1000.0;
        assert_eq!(
            first_visible_block_idx(&laid_out.blocks, &[], way_past),
            laid_out.blocks.len() - 1
        );
    }

    #[test]
    fn culling_excludes_block_above_viewport() {
        let md = "# Top\n\n## Bottom";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
        // Scroll past first heading
        let scroll_y = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h + 10.0;
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        render_doc_with_offset(
            &laid_out,
            &style,
            &mut dl,
            scroll_y,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
        );
        let texts: Vec<&str> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(!texts.iter().any(|t| t.contains("Top")), "block above viewport excluded");
        assert!(texts.iter().any(|t| t.contains("Bottom")), "visible block included");
    }

    #[test]
    fn culling_excludes_block_below_viewport() {
        let md = "# Top\n\n## Bottom";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
        let small_vp = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h + 5.0;
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        render_doc_with_offset(
            &laid_out,
            &style,
            &mut dl,
            0.0,
            small_vp,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
        );
        let texts: Vec<&str> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(!texts.iter().any(|t| t.contains("Bottom")), "block below viewport excluded");
        assert!(texts.iter().any(|t| t.contains("Top")), "visible block included");
    }

    #[test]
    fn blockquote_border_rendered_even_when_child_culled() {
        let md = "> quoted\n> text";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
        let blockquote = &laid_out.blocks[0];
        // Scroll to show only the bottom half of the blockquote
        let scroll_y = blockquote.rect.y + blockquote.rect.h * 0.5;
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        render_doc_with_offset(
            &laid_out,
            &style,
            &mut dl,
            scroll_y,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
        );
        // Blockquote border should still be rendered (the block IS visible, just scrolled)
        let has_border = dl.cmds.iter().any(
            |c| matches!(c, DrawCmd::FillRect { color, .. } if *color == style.blockquote_border),
        );
        assert!(has_border, "blockquote border must render when block is visible");
    }

    #[test]
    fn culling_excludes_codeblock_line_above_viewport() {
        let md = "```\nline1\nline2\nline3\n```";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
        let block = &laid_out.blocks[0];
        // Extract line positions
        if let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind {
            assert!(lines.len() >= 2, "need at least 2 lines");
            // Scroll past first line
            let scroll_y = lines[0].rect.y + lines[0].rect.h + 1.0;
            let mut dl = DrawList::new();
            let mut shaper = shaping::Shaper::new().unwrap();
            render_doc_with_offset(
                &laid_out,
                &style,
                &mut dl,
                scroll_y,
                600.0,
                0.0,
                0.0,
                Some(&mut shaper),
                &[],
            );
            // line1 should not appear, line2/line3 should
            let texts: Vec<&str> = dl
                .cmds
                .iter()
                .filter_map(|c| {
                    if let DrawCmd::TextLayout { layout, .. } = c {
                        Some(layout.text.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            let rendered: String = texts.concat();
            assert!(!rendered.contains("line1"), "line above viewport should be culled");
            assert!(
                rendered.contains("line2") || rendered.contains("line3"),
                "visible lines should render"
            );
        } else {
            panic!("expected CodeBlock");
        }
    }

    #[test]
    fn culling_excludes_listitem_line_above_viewport() {
        let md = "- item one\n- item two\n- item three";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
        assert!(laid_out.blocks.len() >= 2, "need at least 2 list items");
        // Scroll past first item
        let scroll_y = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h + 1.0;
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        render_doc_with_offset(
            &laid_out,
            &style,
            &mut dl,
            scroll_y,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
        );
        let texts: Vec<&str> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let DrawCmd::TextLayout { layout, .. } = c {
                    Some(layout.text.as_str())
                } else {
                    None
                }
            })
            .collect();
        let rendered: String = texts.concat();
        assert!(!rendered.contains("item one"), "list item above viewport should be culled");
        assert!(
            rendered.contains("item two") || rendered.contains("item three"),
            "visible items should render"
        );
    }

    #[test]
    fn empty_doc_renders_nothing() {
        let md = "";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        render_doc_with_offset(
            &laid_out,
            &style,
            &mut dl,
            0.0,
            600.0,
            0.0,
            0.0,
            Some(&mut shaper),
            &[],
        );
        // Should only have PushClip and PopClip
        assert!(
            dl.cmds.len() <= 2,
            "empty doc should produce no draw commands beyond clip, got {}",
            dl.cmds.len()
        );
    }

    #[test]
    fn code_block_uses_border_radius_base() {
        let dl = build_and_render(
            "```
code
```",
        );
        // CodeBlock background should use border_radius_base (8.0), not hardcoded 4.0
        let code_fills: Vec<_> =
            dl.cmds
                .iter()
                .filter_map(|c| {
                    if let DrawCmd::FillRect { radius, .. } = c { Some(*radius) } else { None }
                })
                .collect();
        let style = default_style();
        assert!(
            code_fills.contains(&style.border_radius_base),
            "code block should use border_radius_base={}, got {:?}",
            style.border_radius_base,
            code_fills
        );
    }

    #[test]
    fn code_block_has_stroke_border() {
        let dl = build_and_render(
            "```
code
```",
        );
        let style = default_style();
        let has_border = dl.cmds.iter().any(
            |c| matches!(c, DrawCmd::StrokeRect { color, .. } if *color == style.code_block_border),
        );
        assert!(has_border, "code block should have stroke border with code_block_border color");
    }

    #[test]
    fn inline_code_uses_inline_code_bg() {
        let dl = build_and_render("use `println!` here");
        let style = default_style();
        // inline_code_bg should differ from code_bg
        assert_ne!(
            style.inline_code_bg, style.code_bg,
            "inline_code_bg should differ from code_bg"
        );
        // Should have a fill with inline_code_bg color
        let has_inline_bg = dl.cmds.iter().any(
            |c| matches!(c, DrawCmd::FillRect { color, .. } if *color == style.inline_code_bg),
        );
        assert!(has_inline_bg, "inline code should use inline_code_bg color, not code_bg");
    }

    #[test]
    fn blockquote_border_is_4px_wide() {
        let dl = build_and_render("> quote");
        let style = default_style();
        // Find the blockquote border fill — should be 4px wide
        let border_fills: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let DrawCmd::FillRect { rect, color, radius: _ } = c {
                    if *color == style.blockquote_border { Some(rect.w) } else { None }
                } else {
                    None
                }
            })
            .collect();
        assert!(
            border_fills.contains(&4.0),
            "blockquote border should be 4px wide, got {:?}",
            border_fills
        );
    }

    #[test]
    fn blockquote_bg_covers_full_width() {
        let dl = build_and_render("> quote");
        let style = default_style();
        // The blockquote bg fill should start at the block's x (not x+4)
        // and have the block's full width (not reduced)
        let bg_fills: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let DrawCmd::FillRect { rect, color, radius } = c {
                    if *color == style.blockquote_bg && *radius == style.border_radius_base {
                        Some(rect)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        assert!(!bg_fills.is_empty(), "should have blockquote bg fill");
        // The bg rect width should be > 0 (full-width, not reduced by border)
        assert!(
            bg_fills[0].w > 4.0,
            "blockquote bg should cover full width, got w={}",
            bg_fills[0].w
        );
    }

    #[test]
    fn blockquote_height_includes_vertical_padding() {
        let md = "> line1\n> line2";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
        let blockquote = &laid_out.blocks[0];
        // Block height should include top + bottom padding
        assert!(
            blockquote.rect.h > style.blockquote_padding * 2.0,
            "blockquote height {} should include 2*padding={}",
            blockquote.rect.h,
            style.blockquote_padding * 2.0
        );
        if let LaidOutBlockKind::BlockQuote { blocks } = &blockquote.kind {
            // The internal child blocks should be positioned after top padding
            let first_child_y = blocks[0].rect.y;
            let bq_top = blockquote.rect.y;
            let top_gap = first_child_y - bq_top;
            assert!(
                top_gap >= style.blockquote_padding - 1.0,
                "top gap {} should be >= blockquote_padding {}",
                top_gap,
                style.blockquote_padding
            );
        } else {
            panic!("expected BlockQuote");
        }
    }

    #[test]
    fn list_items_have_uniform_height() {
        let md = "- a\n- b\n- c";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
        assert!(laid_out.blocks.len() >= 3, "need 3 list items");
        // All single-line list items should have the same rect height.
        // Spacing between items is handled externally, not baked into rect.h.
        let first_h = laid_out.blocks[0].rect.h;
        let second_h = laid_out.blocks[1].rect.h;
        let third_h = laid_out.blocks[2].rect.h;
        assert!(
            (second_h - first_h).abs() < 1.0,
            "all list items should have equal height, got {} vs {}",
            first_h,
            second_h
        );
        assert!(
            (third_h - first_h).abs() < 1.0,
            "all list items should have equal height, got {} vs {}",
            first_h,
            third_h
        );
        // Verify inter-item spacing is correct
        let gap_0_1 =
            laid_out.blocks[1].rect.y - (laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h);
        let gap_1_2 =
            laid_out.blocks[2].rect.y - (laid_out.blocks[1].rect.y + laid_out.blocks[1].rect.h);
        assert!(
            (gap_0_1 - style.list_item_spacing).abs() < 1.0,
            "gap between items 0-1 should be ~{}, got {}",
            style.list_item_spacing,
            gap_0_1
        );
        assert!(
            (gap_1_2 - style.list_item_spacing).abs() < 1.0,
            "gap between items 1-2 should be ~{}, got {}",
            style.list_item_spacing,
            gap_1_2
        );
    }

    #[test]
    fn horizontal_rule_centered_and_spaced() {
        let md = "text\n\n---";
        let parsed = parse_markdown(md);
        let style = default_style();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
        // blocks[0] is "text" paragraph, blocks[1] is the HR
        let hr = laid_out
            .blocks
            .iter()
            .find(|b| matches!(b.kind, LaidOutBlockKind::HorizontalRule))
            .expect("should have HorizontalRule block");
        // Height should be 2*rule_spacing + rule_thickness
        let expected_h = style.rule_spacing + style.rule_thickness + style.rule_spacing;
        assert!(
            (hr.rect.h - expected_h).abs() < 1.0,
            "hr height should be ~{}, got {}",
            expected_h,
            hr.rect.h
        );
        // Render and check the fill is centered vertically
        let mut dl = DrawList::new();
        let mut shaper = shaping::Shaper::new().unwrap();
        render_doc(&laid_out, &style, &mut dl, 0.0, 600.0, Some(&mut shaper));
        let rule_fills: Vec<_> = dl
            .cmds
            .iter()
            .filter_map(|c| {
                if let DrawCmd::FillRect { rect, color, .. } = c {
                    if *color == style.rule_color { Some(*rect) } else { None }
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(rule_fills.len(), 1, "should have exactly one rule fill");
        let rule = &rule_fills[0];
        // Rule should be centered: rule.y = block_y + (block_h - rule_thickness) / 2
        let expected_y = hr.rect.y + (hr.rect.h - style.rule_thickness) / 2.0;
        assert!(
            (rule.y - expected_y).abs() < 1.0,
            "rule should be centered at y={}, got {}",
            expected_y,
            rule.y
        );
    }

    #[test]
    fn estimate_text_width_cjk_wider_than_ascii() {
        // CJK chars should be ~1.0 * font_size, ASCII ~0.55 * font_size
        let cjk_w = estimate_text_width("栏目", 15.0); // 2 CJK chars
        let ascii_w = estimate_text_width("ab", 15.0); // 2 ASCII chars
        assert!(cjk_w > ascii_w, "CJK width ({}) should be > ASCII width ({})", cjk_w, ascii_w);
        // CJK: 2 * 15.0 = 30.0; ASCII: 2 * 15.0 * 0.55 = 16.5
        assert!((cjk_w - 30.0).abs() < 0.1, "CJK width should be ~30.0, got {}", cjk_w);
        assert!((ascii_w - 16.5).abs() < 0.1, "ASCII width should be ~16.5, got {}", ascii_w);
    }

    #[test]
    fn estimate_text_width_mixed_cjk_ascii() {
        // Mixed "删除 crash" = 2 CJK + 1 space + 5 ASCII
        let w = estimate_text_width("删除 crash", 15.0);
        // CJK: 2 * 15.0 = 30.0; space: 15.0 * 0.55 = 8.25; ASCII: 5 * 15.0 * 0.55 = 41.25
        let expected = 30.0 + 8.25 + 41.25;
        assert!((w - expected).abs() < 0.1, "mixed width should be ~{}, got {}", expected, w);
    }

    #[test]
    fn estimate_text_width_fullwidth_punctuation() {
        // Fullwidth punctuation (：) is in U+FF01..=U+FF5E range
        let w = estimate_text_width("：", 15.0);
        assert!((w - 15.0).abs() < 0.1, "fullwidth colon should be ~15.0, got {}", w);
    }

    #[test]
    fn render_yaml_metadata_block_emits_fill() {
        let dl = build_and_render(
            "---
title: hello
---",
        );
        // Metadata block should render with code-block-like background
        let has_fill = dl.cmds.iter().any(|c| matches!(c, DrawCmd::FillRect { .. }));
        assert!(has_fill, "YAML metadata block should emit FillRect (background)");
    }
}
