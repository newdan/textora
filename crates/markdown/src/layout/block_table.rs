use super::*;

pub(in crate::layout::block) fn materialize_table_cell_line(
    cell: &BlockNode,
    line_index: usize,
    text: &str,
    styles: &[StyleSpan],
    ctx: &LayoutCtx,
) -> (crate::projection::ProjectedText, Vec<StyleSpan>) {
    let base =
        cell.projected_lines.get(line_index).cloned().unwrap_or_else(|| {
            crate::projection::ProjectedText::direct(text, cell.block_range.start)
        });
    let Some(source) = ctx.source_text else {
        return (base, styles.to_vec());
    };
    let source_line =
        SourceLineContext { range: cell.block_range.clone(), text_start: cell.block_range.start };
    let projected = crate::edit::materialize_projected_line(
        &base,
        styles,
        source,
        ctx.edit_ctx,
        Some(&source_line),
    );
    let materialized_styles = crate::edit::materialized_spans_for_projected_line(
        &base,
        &projected,
        styles,
        source,
        ctx.edit_ctx,
    )
    .into_iter()
    .map(|span| StyleSpan {
        start: span.start,
        len: span.len,
        style: span.style,
        source_range: span.source_range,
    })
    .collect();
    (projected, materialized_styles)
}

pub(crate) fn layout_table(block: &BlockNode, ctx: &mut LayoutCtx, columns: usize) {
    let font_size = ctx.style.body_font_size;
    let line_h = ctx.style.line_height;
    let pad = ctx.style.table_cell_padding;
    let available_w = ctx.available_width().max(20.0);

    // Give short columns enough room for their content, then use the remaining
    // width where wrapping would otherwise be most severe.
    let demand = measure_column_demand(
        block,
        columns,
        font_size,
        ctx.style.text_spacing_mode,
        ctx.style.body_font_family.first().map(String::as_str),
        ctx.shaper.as_deref_mut(),
        ctx.doc,
    );
    const MIN_COLUMN_CHARACTERS: f32 = 3.0;
    let min_col_w = font_size * MIN_COLUMN_CHARACTERS;
    let column_widths = allocate_column_widths(&demand, available_w, pad, min_col_w);

    let table_x = ctx.indent;
    let table_y = ctx.y;
    let table_start = block.block_range.start;
    let mut row_y = table_y;

    let mut header: Vec<Vec<LaidOutLine>> = Vec::new();
    let mut body_rows: Vec<Vec<Vec<LaidOutLine>>> = Vec::new();
    let mut body_row_heights: Vec<f32> = Vec::new();
    let mut header_actual_h = 0.0f32;
    let mut body_rows_h = 0.0f32;

    // Helper closure: layout a single row of cells at the given y position.
    // Returns (laid_out_cells, actual_row_height).
    let layout_cells = |cells: &[&BlockNode],
                        row_y: f32,
                        row_index: usize,
                        column_widths: &[f32],
                        table_x: f32,
                        pad: f32,
                        font_size: f32,
                        line_h: f32,
                        ctx: &mut LayoutCtx|
     -> (Vec<Vec<LaidOutLine>>, f32) {
        let mut row = Vec::new();
        let mut col_x = table_x;
        let row_start_y = row_y;
        let mut max_cell_bottom = row_start_y;
        for (ci, cell) in cells.iter().enumerate() {
            let cell_w = column_widths.get(ci).copied().unwrap_or(0.0);
            let (texts, text_styles) = collect_text_lines_with_styles(cell, ctx.doc);
            let mut laid_out = Vec::new();
            let mut cy = row_y + pad;
            for (t_idx, t) in texts.iter().enumerate() {
                let line_styles = text_styles.get(t_idx).map(|s| s.as_slice()).unwrap_or(&[]);
                let (mut projected, mut materialized_styles) =
                    materialize_table_cell_line(cell, t_idx, t, line_styles, ctx);
                crate::layout::embedded::expose_failed_math(
                    &mut projected,
                    &mut materialized_styles,
                    ctx.doc,
                    font_size,
                    ctx.style.text_color,
                    !ctx.allow_embedded || ctx.shaper.is_none(),
                    ctx.selection_range,
                );
                let cell_x = col_x + pad;
                let cell_inner_w = (cell_w - pad * 2.0).max(1.0);
                let projected_verbatim_ranges = projected_verbatim_ranges(&projected);
                let wrapped = ctx.wrap_text_with_first_line_indent_styled_and_protected(
                    &projected.text,
                    font_size,
                    Weight::NORMAL,
                    cell_inner_w,
                    0.0,
                    &materialized_styles,
                    &projected_verbatim_ranges,
                    ctx.style.text_color,
                );
                let wrapped_shaped = ctx.last_wrap_shaped.first().cloned().flatten();
                let mut laid = layout_line_with_styles(
                    &materialized_styles,
                    &wrapped,
                    &projected,
                    font_size,
                    line_h,
                    cell_x,
                    cy,
                    cell_inner_w,
                    ctx.style.text_color,
                    ctx.style.body_font_family.first().map(|s| s.as_str()),
                    wrapped_shaped.as_ref(),
                    t_idx,
                    &ctx.embedded_images,
                );
                let owner = crate::projection::ProjectionOwnerId::TableCell {
                    table_start,
                    row: row_index,
                    column: ci,
                    logical_line: t_idx,
                };
                for line in &mut laid {
                    line.source_projection
                        .as_mut()
                        .expect("table text layout must retain a source projection")
                        .owner = owner;
                }
                let n = laid.len();
                let laid_height = laid.iter().map(|line| line.rect.h).sum::<f32>();
                laid_out.extend(laid);
                cy += if n == 0 { line_h } else { laid_height };
            }
            if cy > max_cell_bottom {
                max_cell_bottom = cy;
            }
            row.push(laid_out);
            col_x += cell_w;
        }
        let actual_row_h = (max_cell_bottom - row_start_y + pad).max(line_h + 4.0);
        (row, actual_row_h)
    };

    // Pass 1: collect header cells (TableCell_ with is_header=true, direct children)
    let header_cells: Vec<&BlockNode> = block
        .children
        .iter()
        .filter(|c| matches!(c.kind, BlockKind::TableCell_ { is_header: true, .. }))
        .collect();
    if !header_cells.is_empty() {
        let (row, h) = layout_cells(
            &header_cells,
            row_y,
            0,
            &column_widths,
            table_x,
            pad,
            font_size,
            line_h,
            ctx,
        );
        header = row;
        header_actual_h = h;
        row_y += h;
    }

    // Pass 2: process TableRow_ children as body rows
    for (body_row_idx, child) in
        block.children.iter().filter(|child| matches!(child.kind, BlockKind::TableRow_)).enumerate()
    {
        let cell_refs: Vec<&BlockNode> = child.children.iter().collect();
        let (row, actual_row_h) = layout_cells(
            &cell_refs,
            row_y,
            body_row_idx + 1,
            &column_widths,
            table_x,
            pad,
            font_size,
            line_h,
            ctx,
        );
        row_y += actual_row_h;
        body_rows.push(row);
        body_row_heights.push(actual_row_h);
        body_rows_h += actual_row_h;
    }
    let header_h = if header.is_empty() { 0.0 } else { header_actual_h.max(0.0) };
    let total_h = header_h + body_rows_h + 4.0;

    ctx.push_block(
        LaidOutBlockKind::Table {
            columns,
            header,
            rows: body_rows,
            column_widths,
            header_height: header_h,
            row_heights: body_row_heights,
        },
        total_h,
    );
}

/// Collect text lines from a block. Prefers the block's own text_lines;
/// falls back to recursively collecting from children.
pub(crate) fn collect_text_lines<'a>(
    block: &'a BlockNode,
    doc: &'a dyn core::document::DocView,
) -> Vec<std::borrow::Cow<'a, str>> {
    let lines = block.lines(doc);
    if !lines.is_empty() {
        return lines;
    }
    let mut texts = Vec::new();
    for child in &block.children {
        texts.extend(collect_text_lines(child, doc));
    }
    texts
}

/// Collect text lines with their style spans from a block.
pub(crate) fn collect_text_lines_with_styles<'a>(
    block: &'a BlockNode,
    doc: &'a dyn core::document::DocView,
) -> (Vec<std::borrow::Cow<'a, str>>, Vec<Vec<StyleSpan>>) {
    let lines = block.lines(doc);
    if !lines.is_empty() {
        return (lines, block.text_styles.clone());
    }
    let mut texts = Vec::new();
    let mut styles = Vec::new();
    for child in &block.children {
        let (t, s) = collect_text_lines_with_styles(child, doc);
        texts.extend(t);
        styles.extend(s);
    }
    (texts, styles)
}

/// Measure per-column content width demand for dynamic column sizing.
///
/// For each column, computes the maximum of:
///   - the longest non-breakable token width (spaces and CJK characters break)
///   - the longest full-line width × 0.6
///
/// This ensures narrow content columns don't hog space while wide columns
/// get enough room to avoid excessive wrapping.
pub(crate) fn measure_column_demand(
    block: &BlockNode,
    columns: usize,
    font_size: f32,
    text_spacing_mode: ui::typography::TextSpacingMode,
    font_family: Option<&str>,
    mut shaper: Option<&mut Shaper>,
    doc: &dyn core::document::DocView,
) -> Vec<f32> {
    let mut demand = vec![0.0f32; columns];

    // Helper: update demand for a single cell's text content.
    let measure_cell = |texts: &[std::borrow::Cow<'_, str>],
                        styles: &[Vec<StyleSpan>],
                        projected_verbatim: &[Vec<std::ops::Range<usize>>],
                        ci: usize,
                        demand: &mut Vec<f32>,
                        shaper: &mut Option<&mut Shaper>,
                        font_size: f32| {
        for (line_index, t) in texts.iter().enumerate() {
            if t.is_empty() {
                continue;
            }
            let (max_token_w, full_w) = shaper
                .as_mut()
                .map(|s| {
                    s.set_font_size(font_size);
                    let max_tok = t
                        .split(|character: char| {
                            character == ' '
                                || crate::layout::context::is_cjk_or_fullwidth(character)
                        })
                        .filter_map(|tok| shape_table_demand_token(tok, font_size, font_family, s))
                        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                        .unwrap_or(0.0)
                        .max(if t.chars().any(crate::layout::context::is_cjk_or_fullwidth) {
                            font_size
                        } else {
                            0.0
                        });
                    let full = measure_styled_text_width(
                        t,
                        styles.get(line_index).map(Vec::as_slice).unwrap_or(&[]),
                        font_size,
                        text_spacing_mode,
                        font_family,
                        projected_verbatim.get(line_index).map(Vec::as_slice).unwrap_or(&[]),
                        s,
                    );
                    (max_tok, full)
                })
                .unwrap_or_else(|| {
                    let char_est = |tok: &str| tok.chars().count() as f32 * font_size * 0.55;
                    let max_tok = t
                        .split(|character: char| {
                            character == ' '
                                || crate::layout::context::is_cjk_or_fullwidth(character)
                        })
                        .map(&char_est)
                        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                        .unwrap_or(0.0)
                        .max(if t.chars().any(crate::layout::context::is_cjk_or_fullwidth) {
                            font_size
                        } else {
                            0.0
                        });
                    (max_tok, char_est(t))
                });
            let image_width = styles
                .get(line_index)
                .into_iter()
                .flatten()
                .filter(|_| shaper.is_some())
                .filter(|span| span.style == crate::builder::InlineStyle::Math)
                .filter_map(|span| {
                    let spelling = doc.doc_text_in_range(span.source_range.clone());
                    let expression = spelling.strip_prefix('$')?.strip_suffix('$')?;
                    let image =
                        crate::embedded::render_embedded(crate::embedded::EmbeddedRequest {
                            kind: crate::embedded::EmbeddedKind::InlineMath,
                            source: expression,
                            font_size,
                            foreground: [0, 0, 0, 255],
                            background: [0, 0, 0, 0],
                        })
                        .ok()?;
                    Some(image.image.width() as f32)
                })
                .fold(0.0f32, f32::max);
            let d = max_token_w.max(full_w * 0.6).max(image_width);
            if d > demand[ci] {
                demand[ci] = d;
            }
        }
    };

    // Pass 1: header cells (TableCell_ with is_header=true, direct children)
    for child in &block.children {
        if let BlockKind::TableCell_ { col, is_header: true, .. } = child.kind
            && col < columns
        {
            let (texts, styles) = collect_text_lines_with_styles(child, doc);
            let projected_verbatim =
                child.projected_lines.iter().map(projected_verbatim_ranges).collect::<Vec<_>>();
            measure_cell(
                &texts,
                &styles,
                &projected_verbatim,
                col,
                &mut demand,
                &mut shaper,
                font_size,
            );
        }
    }

    // Pass 2: body row cells (TableRow_ children)
    for child in &block.children {
        if !matches!(child.kind, BlockKind::TableRow_) {
            continue;
        }
        for (ci, cell) in child.children.iter().enumerate() {
            if ci >= columns {
                break;
            }
            let (texts, styles) = collect_text_lines_with_styles(cell, doc);
            let projected_verbatim =
                cell.projected_lines.iter().map(projected_verbatim_ranges).collect::<Vec<_>>();
            measure_cell(
                &texts,
                &styles,
                &projected_verbatim,
                ci,
                &mut demand,
                &mut shaper,
                font_size,
            );
        }
    }
    demand
}

pub(in crate::layout::block) fn shape_table_demand_token(
    text: &str,
    font_size: f32,
    font_family: Option<&str>,
    shaper: &mut Shaper,
) -> Option<f32> {
    let previous_font_size = shaper.font_size();
    let previous_font_weight = shaper.font_weight();
    let previous_font_style = shaper.font_style();
    let previous_font_family = shaper.font_family().map(str::to_owned);

    shaper.set_font_size(font_size);
    shaper.set_font_weight(Weight::NORMAL);
    shaper.set_font_style(shaping::Style::Normal);
    shaper.set_font_family(font_family);
    let width = shaper.shape(text).ok().map(|shaped| shaped.width);

    shaper.set_font_size(previous_font_size);
    shaper.set_font_weight(previous_font_weight);
    shaper.set_font_style(previous_font_style);
    shaper.set_font_family(previous_font_family.as_deref());

    width
}

pub(in crate::layout::block) fn measure_styled_text_width(
    text: &str,
    styles: &[StyleSpan],
    font_size: f32,
    text_spacing_mode: ui::typography::TextSpacingMode,
    font_family: Option<&str>,
    projected_verbatim: &[std::ops::Range<usize>],
    shaper: &mut Shaper,
) -> f32 {
    let Some(shaped) = crate::layout::shaping::shape_styled_run(
        text,
        styles,
        font_size,
        Weight::NORMAL,
        font_family,
        shaper,
    ) else {
        return 0.0;
    };
    let clusters = shaped
        .clusters
        .iter()
        .map(|cluster| {
            ui::typography::TypographyCluster::new(cluster.byte_range.clone(), font_size)
        })
        .collect::<Vec<_>>();
    let mut protected_ranges = styles
        .iter()
        .filter(|span| {
            matches!(
                span.style,
                crate::builder::InlineStyle::InlineCode | crate::builder::InlineStyle::SourceMarker
            )
        })
        .map(|span| span.start..span.start + span.len)
        .collect::<Vec<_>>();
    protected_ranges.extend(projected_verbatim.iter().cloned());
    let gaps = ui::typography::calculate_gaps(&ui::typography::TypographyInput::new(
        text,
        text_spacing_mode,
        &clusters,
        &protected_ranges,
    ));
    shaped.width + gaps.iter().map(|gap| gap.advance).sum::<f32>()
}

/// Allocate column widths from content demand and available space.
///
/// Each column first gets its minimum width. When content does not fit, remaining
/// space follows unmet demand; otherwise spare space is shared evenly.
pub(crate) fn allocate_column_widths(
    demand: &[f32],
    available_w: f32,
    pad: f32,
    min_col_w: f32,
) -> Vec<f32> {
    let cols = demand.len();
    if cols == 0 {
        return vec![];
    }

    // Single column: use full available width
    if cols == 1 {
        return vec![available_w];
    }

    let cell_padding = pad * 2.0;
    let content_budget = (available_w - cell_padding * cols as f32).max(0.0);
    let minimum_total = min_col_w * cols as f32;
    if content_budget <= minimum_total {
        return vec![available_w / cols as f32; cols];
    }

    let content_targets: Vec<f32> = demand.iter().map(|&width| width.max(min_col_w)).collect();
    let target_total: f32 = content_targets.iter().sum();
    if target_total <= content_budget {
        let spare_per_column = (content_budget - target_total) / cols as f32;
        return content_targets
            .into_iter()
            .map(|width| width + spare_per_column + cell_padding)
            .collect();
    }

    let remaining = content_budget - minimum_total;
    let unmet_total = target_total - minimum_total;
    content_targets
        .into_iter()
        .map(|width| min_col_w + remaining * (width - min_col_w) / unmet_total + cell_padding)
        .collect()
}
