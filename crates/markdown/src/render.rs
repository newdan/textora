//! Render pass — converts LaidOutDoc into DrawList commands.

pub(in crate::render) use std::sync::Arc;

pub(in crate::render) use ui::core::geom::Rect;
pub(in crate::render) use ui::core::paint::{DrawCmd, DrawList};
pub(in crate::render) use ui::core::text_layout::UiTextLayout;

pub(in crate::render) use crate::builder::{InlineStyle, ListBullet};
pub(in crate::render) use crate::layout::block::MarkdownLayout;
pub(in crate::render) use crate::layout::embedded::EmbeddedRegistry;
pub(in crate::render) use crate::layout::{
    AsciiDiagramRegistry, AsciiDiagramRow, BoxConnections, LaidOutBlock, LaidOutBlockKind,
    LaidOutDoc, LaidOutLine,
};
#[cfg(test)]
pub(in crate::render) use crate::layout::{BlockSource, LazyLayout};
pub(in crate::render) use crate::safe_byte_idx;
pub(in crate::render) use crate::style::{MarkdownStyle, blend_toward_bg};
pub(in crate::render) use shaping::{Style, Weight};
pub(in crate::render) use ui::core::text_layout::ITALIC_SHEAR;

pub(in crate::render) const SOURCE_MARKER_FADE_RATIO: f32 = 0.55;
pub(in crate::render) const INLINE_CODE_BACKGROUND_HEIGHT_RATIO: f32 = 1.3;
pub(in crate::render) const INLINE_CODE_BACKGROUND_RADIUS_RATIO: f32 = 0.35;
pub(in crate::render) const INLINE_CODE_BACKGROUND_HORIZONTAL_PADDING_RATIO: f32 = 0.25;
pub(in crate::render) const STRIKETHROUGH_THICKNESS_RATIO: f32 = 1.0 / 15.0;
pub(in crate::render) const MIN_STRIKETHROUGH_THICKNESS: f32 = 1.0;

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
#[allow(clippy::too_many_arguments)]
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
#[allow(clippy::too_many_arguments)]
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
    render_doc_with_offset_and_embedded(
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
        Some(layout.embedded_images()),
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_doc_with_offset_and_ascii_diagrams(
    doc: &LaidOutDoc,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    viewport_h: f32,
    offset_x: f32,
    offset_y: f32,
    shaper: Option<&mut shaping::Shaper>,
    y_delta: &[f32],
    ascii_diagrams: Option<&AsciiDiagramRegistry>,
) {
    render_doc_with_offset_and_embedded(
        doc,
        style,
        dl,
        scroll_y,
        viewport_h,
        offset_x,
        offset_y,
        shaper,
        y_delta,
        ascii_diagrams,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_doc_with_offset_and_embedded(
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
    embedded_images: Option<&EmbeddedRegistry>,
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
            embedded_images,
        );
    }

    dl.cmds.push(DrawCmd::PopClip);
}

#[allow(clippy::too_many_arguments)]
pub(in crate::render) fn render_block_with_offset(
    block: &LaidOutBlock,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    viewport_h: f32,
    ox: f32,
    oy: f32,
    shaper: Option<&mut shaping::Shaper>,
    ascii_diagrams: Option<&AsciiDiagramRegistry>,
    embedded_images: Option<&EmbeddedRegistry>,
) {
    let paint = BlockPaint { style, scroll_y, viewport_h, ox, oy, ascii_diagrams, embedded_images };
    paint.draw(block, dl, shaper);
}

struct BlockPaint<'a> {
    style: &'a MarkdownStyle,
    scroll_y: f32,
    viewport_h: f32,
    ox: f32,
    oy: f32,
    ascii_diagrams: Option<&'a AsciiDiagramRegistry>,
    embedded_images: Option<&'a EmbeddedRegistry>,
}

struct TablePaint<'a> {
    columns: usize,
    header: &'a [Vec<LaidOutLine>],
    rows: &'a [Vec<Vec<LaidOutLine>>],
    column_widths: &'a [f32],
    header_height: f32,
    row_heights: &'a [f32],
}

impl BlockPaint<'_> {
    fn draw(
        &self,
        block: &LaidOutBlock,
        dl: &mut DrawList,
        mut shaper: Option<&mut shaping::Shaper>,
    ) {
        let rect = Rect::new(
            block.rect.x + self.ox,
            block.rect.y - self.scroll_y + self.oy,
            block.rect.w,
            block.rect.h,
        );
        match &block.kind {
            LaidOutBlockKind::Text { lines } => {
                self.draw_visible_lines(lines, dl, shaper, self.embedded_images);
            }
            LaidOutBlockKind::CodeBlock { lines, .. } => {
                let diagram = self.ascii_diagrams.and_then(|registry| registry.diagram_for(lines));
                self.draw_code_lines(rect, lines, diagram, dl, shaper);
            }
            LaidOutBlockKind::MetadataBlock { lines } => {
                self.draw_code_lines(rect, lines, None, dl, shaper);
            }
            LaidOutBlockKind::BlockQuote { blocks } => self.draw_quote(rect, blocks, dl, shaper),
            LaidOutBlockKind::ListItem { bullet, blocks, lines, depth, .. } => {
                self.draw_list_marker(rect, bullet, lines, *depth, dl, shaper.as_deref_mut());
                self.draw_visible_lines(lines, dl, shaper.as_deref_mut(), self.embedded_images);
                self.draw_children(blocks, dl, shaper);
            }
            LaidOutBlockKind::Table {
                columns,
                header,
                rows,
                column_widths,
                header_height,
                row_heights,
            } => {
                self.draw_table(
                    rect,
                    TablePaint {
                        columns: *columns,
                        header,
                        rows,
                        column_widths,
                        header_height: *header_height,
                        row_heights,
                    },
                    dl,
                    shaper,
                );
            }
            LaidOutBlockKind::HorizontalRule => {
                let width = rect.w * self.style.rule_width_ratio;
                let x = rect.x + (rect.w - width) / 2.0;
                dl.fill(Rect::new(x, rect.y, width, rect.h), self.style.rule_color);
            }
            LaidOutBlockKind::Embedded { source_range } => {
                if let Some(entry) =
                    self.embedded_images.and_then(|images| images.image_for(source_range))
                {
                    let width = (entry.image.width() as f32).min(rect.w);
                    dl.image(entry.image.clone(), Rect::new(rect.x, rect.y, width, rect.h));
                }
            }
        }
    }

    fn draw_visible_lines(
        &self,
        lines: &[LaidOutLine],
        dl: &mut DrawList,
        mut shaper: Option<&mut shaping::Shaper>,
        embedded_images: Option<&EmbeddedRegistry>,
    ) {
        for line in lines {
            if line.rect.y + line.rect.h < self.scroll_y {
                continue;
            }
            if line.rect.y > self.scroll_y + self.viewport_h {
                break;
            }
            render_line_with_embedded(
                line,
                self.style,
                dl,
                self.scroll_y,
                self.ox,
                self.oy,
                shaper.as_deref_mut(),
                embedded_images,
            );
        }
    }

    fn draw_children(
        &self,
        blocks: &[LaidOutBlock],
        dl: &mut DrawList,
        mut shaper: Option<&mut shaping::Shaper>,
    ) {
        for child in blocks {
            if child.rect.y + child.rect.h < self.scroll_y {
                continue;
            }
            if child.rect.y > self.scroll_y + self.viewport_h {
                break;
            }
            self.draw(child, dl, shaper.as_deref_mut());
        }
    }

    fn draw_code_lines(
        &self,
        rect: Rect,
        lines: &[LaidOutLine],
        diagram: Option<&crate::layout::AsciiDiagram>,
        dl: &mut DrawList,
        mut shaper: Option<&mut shaping::Shaper>,
    ) {
        let Self { style, scroll_y, viewport_h, ox, oy, .. } = *self;
        // Background
        dl.fill_rounded(rect, style.code_bg, style.border_radius_base);
        // Border
        dl.stroke_rounded(rect, style.code_block_border, style.border_radius_base, 1.0);
        // Clipped code text
        dl.clip(rect, |dl| {
            if let (Some(diagram), Some(shaper)) = (diagram, shaper.as_deref_mut())
                && diagram.rows.len() == lines.len()
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
                self.draw_visible_lines(lines, dl, shaper, None);
            }
        });
    }

    fn draw_quote(
        &self,
        rect: Rect,
        blocks: &[LaidOutBlock],
        dl: &mut DrawList,
        shaper: Option<&mut shaping::Shaper>,
    ) {
        let style = self.style;
        // Painter's algorithm: background first (full width), then border on top
        dl.fill_rounded(rect, style.blockquote_bg, style.border_radius_base);
        dl.fill(Rect::new(rect.x, rect.y, 4.0, rect.h), style.blockquote_border);
        self.draw_children(blocks, dl, shaper);
    }

    fn draw_list_marker(
        &self,
        rect: Rect,
        bullet: &ListBullet,
        lines: &[LaidOutLine],
        depth: usize,
        dl: &mut DrawList,
        mut shaper: Option<&mut shaping::Shaper>,
    ) {
        let Self { style, scroll_y, oy, .. } = *self;
        let x = rect.x;
        let y = rect.y;
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
                        let actual_w =
                            dl.text_shaped(box_x, check_baseline, check_fs, border_color, "x", s);
                        if let DrawCmd::TextLayout { ref mut x, .. } = dl.cmds[idx] {
                            *x = box_x + (box_size - actual_w) / 2.0;
                        }
                    }
                }
            }
        }
    }

    fn draw_table(
        &self,
        rect: Rect,
        table: TablePaint<'_>,
        dl: &mut DrawList,
        mut shaper: Option<&mut shaping::Shaper>,
    ) {
        let Self { style, scroll_y, ox, oy, embedded_images, .. } = *self;
        let TablePaint { columns, header, rows, column_widths, header_height, row_heights } = table;
        let mut cell_y = rect.y;

        // Header — use actual measured height
        if !header.is_empty() && header_height > 0.0 {
            dl.fill_rounded(
                Rect::new(rect.x, cell_y, rect.w, header_height),
                style.table_header_bg,
                0.0,
            );
            for cell_lines in header.iter() {
                for line in cell_lines {
                    render_line_with_embedded(
                        line,
                        style,
                        dl,
                        scroll_y,
                        ox,
                        oy,
                        shaper.as_deref_mut(),
                        embedded_images,
                    );
                }
            }
            cell_y += header_height;
            // Separator line
            dl.fill(Rect::new(rect.x, cell_y, rect.w, 1.0), style.table_border);
        }

        // Body rows with zebra stripes — use actual measured heights
        for (row_idx, row) in rows.iter().enumerate() {
            let row_h = row_heights.get(row_idx).copied().unwrap_or(style.line_height + 2.0);
            // Zebra stripe: odd rows get a subtle background
            if row_idx % 2 == 1 {
                dl.fill_rounded(
                    Rect::new(rect.x, cell_y, rect.w, row_h),
                    style.table_stripe_bg,
                    0.0,
                );
            }
            for cell_lines in row.iter() {
                for line in cell_lines {
                    render_line_with_embedded(
                        line,
                        style,
                        dl,
                        scroll_y,
                        ox,
                        oy,
                        shaper.as_deref_mut(),
                        embedded_images,
                    );
                }
            }
            cell_y += row_h;
            // Row separator
            dl.fill(Rect::new(rect.x, cell_y, rect.w, 1.0), style.table_border);
        }

        // Vertical grid lines
        for i in 1..columns {
            let cx = rect.x + column_widths[..i].iter().sum::<f32>();
            dl.fill(Rect::new(cx, rect.y, 1.0, rect.h), style.table_border);
        }
    }
}

pub(in crate::render) fn list_item_uses_source_marker(lines: &[LaidOutLine]) -> bool {
    lines.first().is_some_and(|line| {
        line.styles
            .iter()
            .any(|style| style.start == 0 && matches!(style.style, InlineStyle::SourceMarker))
    })
}

pub(in crate::render) fn code_cell_width(
    shaper: &mut shaping::Shaper,
    font_size: f32,
    family: Option<&str>,
) -> f32 {
    let old_size = shaper.font_size();
    let old_family = shaper.font_family().map(str::to_owned);
    shaper.set_font_size(font_size);
    shaper.set_font_family(family);
    let width = shaper.col_width();
    shaper.set_font_size(old_size);
    shaper.set_font_family(old_family.as_deref());
    width
}

#[allow(clippy::too_many_arguments)]
pub(in crate::render) fn draw_box_connections(
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
pub(in crate::render) enum HorizontalArrowDirection {
    Left,
    Right,
}

pub(in crate::render) fn horizontal_arrow_direction(
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

#[allow(clippy::too_many_arguments)]
pub(in crate::render) fn draw_horizontal_arrow(
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

#[allow(clippy::too_many_arguments)]
pub(in crate::render) fn render_ascii_diagram_row(
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

pub(in crate::render) fn highlight_color_for_cell(
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

/// Draw each resolved inter-block gap once, using the published block positions.
/// Block classification affects labels and colors only, never spacing geometry.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_debug_spacing<S: BlockSource>(
    layout: &LazyLayout<S>,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    viewport_h: f32,
    offset_x: f32,
    offset_y: f32,
    shaper: &mut shaping::Shaper,
) {
    const OUTLINE_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.15];
    const OUTLINE_THICKNESS: f32 = 0.5;
    let viewport_bottom = scroll_y + viewport_h;
    for (index, block) in layout.laid_out.iter().enumerate() {
        let Some(block) = block else { continue };
        let y_correction = layout.y_delta.get(index).copied().unwrap_or(0.0);
        let top = block.rect.y + y_correction;
        let previous_bottom = index.checked_sub(1).map_or(0.0, |previous_index| {
            layout.estimated_positions[previous_index]
                + layout.estimated_heights[previous_index]
                + layout.y_delta.get(previous_index).copied().unwrap_or(0.0)
        });
        if previous_bottom > viewport_bottom {
            break;
        }
        if top + block.rect.h < scroll_y {
            continue;
        }
        let (color, label) = debug_block_appearance(block, style);
        let screen_y = top - scroll_y + offset_y;
        let gap = (top - previous_bottom).max(0.0);
        let band = Rect::new(block.rect.x + offset_x, screen_y - gap, block.rect.w, gap);
        draw_debug_spacing_band(dl, band, color, label, style, shaper);
        dl.stroke(
            Rect::new(block.rect.x + offset_x, screen_y, block.rect.w, block.rect.h),
            OUTLINE_COLOR,
            OUTLINE_THICKNESS,
        );
        if index + 1 == layout.estimated_heights.len() {
            let trailing_gap = (layout.total_height - top - block.rect.h).max(0.0);
            let trailing_band = Rect::new(
                block.rect.x + offset_x,
                screen_y + block.rect.h,
                block.rect.w,
                trailing_gap,
            );
            draw_debug_spacing_band(dl, trailing_band, color, "End", style, shaper);
        }
    }
}

#[cfg(test)]
pub(in crate::render) fn debug_block_appearance(
    block: &LaidOutBlock,
    style: &MarkdownStyle,
) -> ([f32; 4], &'static str) {
    const HEADING_LABELS: [&str; 6] = ["H1", "H2", "H3", "H4", "H5", "H6"];
    const HEADING_COLOR: [f32; 4] = [0.2, 0.55, 1.0, 0.85];
    const PARAGRAPH_COLOR: [f32; 4] = [0.3, 0.8, 0.3, 0.8];
    const CODE_COLOR: [f32; 4] = [0.9, 0.5, 0.2, 0.8];
    const LIST_COLOR: [f32; 4] = [0.8, 0.3, 0.8, 0.8];
    const QUOTE_COLOR: [f32; 4] = [0.9, 0.9, 0.2, 0.8];
    const TABLE_COLOR: [f32; 4] = [0.2, 0.8, 0.8, 0.8];
    const RULE_COLOR: [f32; 4] = [0.8, 0.2, 0.2, 0.8];
    const METADATA_COLOR: [f32; 4] = [0.5, 0.5, 0.5, 0.8];
    const FONT_SIZE_TOLERANCE: f32 = 0.01;
    match &block.kind {
        LaidOutBlockKind::Text { lines } => {
            let heading = lines
                .first()
                .filter(|line| line.font_size > style.body_font_size)
                .and_then(|line| {
                    style
                        .heading_font_sizes
                        .iter()
                        .position(|size| (line.font_size - size).abs() < FONT_SIZE_TOLERANCE)
                });
            heading
                .map_or((PARAGRAPH_COLOR, "Para"), |level| (HEADING_COLOR, HEADING_LABELS[level]))
        }
        LaidOutBlockKind::CodeBlock { .. } => (CODE_COLOR, "Code"),
        LaidOutBlockKind::BlockQuote { .. } => (QUOTE_COLOR, "Quote"),
        LaidOutBlockKind::ListItem { .. } => (LIST_COLOR, "List"),
        LaidOutBlockKind::Table { .. } => (TABLE_COLOR, "Table"),
        LaidOutBlockKind::HorizontalRule => (RULE_COLOR, "HR"),
        LaidOutBlockKind::Embedded { .. } => (CODE_COLOR, "Embedded"),
        LaidOutBlockKind::MetadataBlock { .. } => (METADATA_COLOR, "Meta"),
    }
}

#[cfg(test)]
pub(in crate::render) fn draw_debug_spacing_band(
    dl: &mut DrawList,
    band: Rect,
    color: [f32; 4],
    label: &str,
    style: &MarkdownStyle,
    shaper: &mut shaping::Shaper,
) {
    const MINIMUM_BAND_HEIGHT: f32 = 0.5;
    const LABEL_FONT_RATIO: f32 = 0.85;
    const LABEL_INSET_X: f32 = 4.0;
    const LABEL_INSET_Y: f32 = 2.0;
    const LABEL_COLOR: [f32; 4] = [0.95, 0.5, 0.15, 1.0];
    if band.h <= MINIMUM_BAND_HEIGHT {
        return;
    }
    let font_size = style.body_font_size * LABEL_FONT_RATIO;
    dl.fill(band, color);
    dl.text_shaped(
        band.x + LABEL_INSET_X,
        band.y + font_size + LABEL_INSET_Y,
        font_size,
        LABEL_COLOR,
        &format!("{label} {:.0}", band.h),
        shaper,
    );
}

#[path = "render_line.rs"]
mod render_line;
#[allow(unused_imports)]
pub(in crate::render) use render_line::*;

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "render_spacing_tests.rs"]
mod spacing_tests;
