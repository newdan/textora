//! Block-level layout functions.

pub(in crate::layout::block) use shaping::{Shaper, Weight};

pub(in crate::layout::block) use super::context::LayoutCtx;
pub(in crate::layout::block) use super::types::{LaidOutBlock, LaidOutBlockKind, LaidOutLine};
pub(in crate::layout::block) use crate::builder::{
    BlockKind, BlockNode, BlockSourceSpan, StyleSpan,
};
pub(in crate::layout::block) use crate::edit::SourceLineContext;
pub(in crate::layout::block) use crate::style::MarkdownStyle;

pub(in crate::layout::block) const HEADING_LINE_HEIGHT_RATIO: f32 = 1.3;

// ===== Public API =====

/// Layout a list of `BlockNode`s into positioned blocks.
///
/// This compatibility API returns only [`LaidOutDoc`]. Use
/// [`layout_doc_for_rendering`] with [`crate::render::render_layout`] when
/// optional Markdown render metadata, such as ASCII diagram grids, must be
/// preserved through rendering.
pub fn layout_doc(
    blocks: &[BlockNode],
    style: &MarkdownStyle,
    viewport_w: f32,
    text_doc: &dyn textora_core::document::DocView,
) -> LaidOutDoc {
    layout_doc_with_shaper(blocks, style, viewport_w, None, None, text_doc)
}

/// Layout with an optional shaper for precise text measurement.
///
/// This compatibility API returns only [`LaidOutDoc`]. Use
/// [`layout_doc_with_shaper_for_rendering`] with
/// [`crate::render::render_layout`] when optional Markdown render metadata
/// must be preserved through rendering.
pub fn layout_doc_with_shaper(
    blocks: &[BlockNode],
    style: &MarkdownStyle,
    viewport_w: f32,
    shaper: Option<&mut Shaper>,
    highlighter: Option<&dyn crate::builder::CodeHighlighter>,
    text_doc: &dyn textora_core::document::DocView,
) -> LaidOutDoc {
    layout_doc_with_options(blocks, style, viewport_w, shaper, highlighter, text_doc, false).doc
}

/// A Markdown layout that retains crate-private render sidecars.
///
/// Its [`LaidOutDoc`] remains available through [`Self::document`], while
/// Markdown-specific metadata stays encapsulated so UI consumers do not need
/// to name or depend on Markdown grid types.
pub struct MarkdownLayout {
    pub(crate) doc: LaidOutDoc,
    pub(crate) ascii_diagrams: super::ascii_diagram::AsciiDiagramRegistry,
    pub(crate) embedded_images: super::embedded::EmbeddedRegistry,
}

impl MarkdownLayout {
    /// Returns the positioned document for consumers that only need layout data.
    pub fn document(&self) -> &LaidOutDoc {
        &self.doc
    }

    pub(crate) fn ascii_diagrams(&self) -> &super::ascii_diagram::AsciiDiagramRegistry {
        &self.ascii_diagrams
    }

    pub(crate) fn embedded_images(&self) -> &super::embedded::EmbeddedRegistry {
        &self.embedded_images
    }
}

/// Layout a document for rendering while retaining Markdown render sidecars.
pub fn layout_doc_for_rendering(
    blocks: &[BlockNode],
    style: &MarkdownStyle,
    viewport_w: f32,
    text_doc: &dyn textora_core::document::DocView,
) -> MarkdownLayout {
    layout_doc_with_shaper_for_rendering(blocks, style, viewport_w, None, None, text_doc)
}

/// Layout with an optional shaper while retaining Markdown render sidecars.
pub fn layout_doc_with_shaper_for_rendering(
    blocks: &[BlockNode],
    style: &MarkdownStyle,
    viewport_w: f32,
    shaper: Option<&mut Shaper>,
    highlighter: Option<&dyn crate::builder::CodeHighlighter>,
    text_doc: &dyn textora_core::document::DocView,
) -> MarkdownLayout {
    layout_doc_with_options(blocks, style, viewport_w, shaper, highlighter, text_doc, true)
}

pub(in crate::layout::block) fn layout_doc_with_options(
    blocks: &[BlockNode],
    style: &MarkdownStyle,
    viewport_w: f32,
    shaper: Option<&mut Shaper>,
    highlighter: Option<&dyn crate::builder::CodeHighlighter>,
    text_doc: &dyn textora_core::document::DocView,
    allow_embedded: bool,
) -> MarkdownLayout {
    let mut ctx = LayoutCtx::new(text_doc, style, viewport_w, shaper, highlighter, None, None);
    ctx.allow_embedded = allow_embedded;

    for block in blocks {
        layout_block(block, &mut ctx);
    }
    ctx.finish_document_spacing();
    if let Some(shaper) = ctx.shaper.take() {
        for block in &mut ctx.output {
            super::shaping::populate_style_segments(block, shaper, style);
        }
    }

    MarkdownLayout {
        doc: super::types::LaidOutDoc { blocks: ctx.output, total_height: ctx.y },
        ascii_diagrams: ctx.ascii_diagrams,
        embedded_images: ctx.embedded_images,
    }
}

pub(crate) fn layout_doc_for_estimation(
    blocks: &[BlockNode],
    style: &MarkdownStyle,
    viewport_w: f32,
    text_doc: &dyn textora_core::document::DocView,
) -> LaidOutDoc {
    let mut ctx = LayoutCtx::new(text_doc, style, viewport_w, None, None, None, None);
    ctx.allow_embedded = false;
    for block in blocks {
        layout_block(block, &mut ctx);
    }
    ctx.finish_document_spacing();
    LaidOutDoc { blocks: ctx.output, total_height: ctx.y }
}

use super::types::LaidOutDoc;

pub(crate) fn layout_block(block: &BlockNode, ctx: &mut LayoutCtx) {
    layout_block_with_placement(block, ctx, BlockPlacement::InFlow);
}

/// Layout a block whose content origin has already been resolved by its caller.
///
/// This is the precision-layout entry point: `ctx.y` is the content top, so the
/// block's external boundary is intentionally not applied again. Transparent
/// containers pass that origin through to their first actual output block.
pub(crate) fn layout_block_at_content_origin(block: &BlockNode, ctx: &mut LayoutCtx) {
    layout_block_with_placement(block, ctx, BlockPlacement::AtContentOrigin);
}

#[derive(Clone, Copy)]
pub(in crate::layout::block) enum BlockPlacement {
    InFlow,
    AtContentOrigin,
}

pub(in crate::layout::block) fn layout_block_with_placement(
    block: &BlockNode,
    ctx: &mut LayoutCtx,
    placement: BlockPlacement,
) -> bool {
    let Some(spacing_block) = super::spacing::BlockSpacing::from_block_kind(&block.kind) else {
        return layout_transparent_container(block, ctx, placement);
    };
    if matches!(placement, BlockPlacement::InFlow) {
        ctx.advance_to_block(spacing_block);
    }

    layout_block_content(block, ctx);
    ctx.finish_block(spacing_block);
    true
}

pub(in crate::layout::block) fn layout_transparent_container(
    block: &BlockNode,
    ctx: &mut LayoutCtx,
    placement: BlockPlacement,
) -> bool {
    let mut next_placement = placement;
    let mut produced_output = false;
    for child in &block.children {
        if layout_block_with_placement(child, ctx, next_placement) {
            next_placement = BlockPlacement::InFlow;
            produced_output = true;
        }
    }
    produced_output
}

pub(in crate::layout::block) fn layout_block_content(block: &BlockNode, ctx: &mut LayoutCtx) {
    match &block.kind {
        BlockKind::Container | BlockKind::TableRow_ | BlockKind::TableCell_ { .. } => {
            unreachable!("transparent containers are handled before content layout")
        }
        BlockKind::Paragraph => {
            let font_size = ctx.font_size_override.unwrap_or(ctx.style.body_font_size);
            layout_text_block(block, ctx, font_size, ctx.style.text_color, Weight::NORMAL);
        }
        BlockKind::DisplayMath => layout_display_math_block(block, ctx),
        BlockKind::Heading { level } => layout_heading_block(block, ctx, *level),
        BlockKind::CodeBlock { language } => layout_code_block(block, ctx, language.as_deref()),
        BlockKind::BlockQuote => layout_quote_block(block, ctx),
        BlockKind::ListItem { bullet, .. } => layout_list_item_block(block, ctx, bullet),
        BlockKind::TableWrapper { columns, alignments: _ } => {
            layout_table(block, ctx, *columns);
        }
        BlockKind::HorizontalRule => layout_horizontal_rule(block, ctx),
        BlockKind::MetadataBlock => layout_metadata_block(block, ctx),
    }
}

fn layout_display_math_block(block: &BlockNode, ctx: &mut LayoutCtx) {
    let active = block_is_active(block, ctx);
    if !active {
        let spelling = ctx.doc.doc_text_in_range(block.block_range.clone());
        let expression =
            spelling.trim().strip_prefix("$$").and_then(|body| body.strip_suffix("$$"));
        if let Some(expression) = expression
            && layout_embedded_block(
                block,
                ctx,
                crate::embedded::EmbeddedKind::DisplayMath,
                expression.trim(),
            )
        {
            return;
        }
    }
    let font_size = ctx.font_size_override.unwrap_or(ctx.style.body_font_size);
    layout_text_block(block, ctx, font_size, ctx.style.text_color, Weight::NORMAL);
}

fn layout_heading_block(block: &BlockNode, ctx: &mut LayoutCtx, level: u8) {
    let idx = (level as usize).saturating_sub(1).min(5);
    let font_size = ctx.style.heading_font_sizes[idx];
    if let (Some(edit_ctx), Some(source)) = (ctx.edit_ctx, ctx.source_text) {
        ctx.active_block_marker = heading_marker_at_cursor(block, source, edit_ctx.cursor_byte);
    }
    layout_text_block(block, ctx, font_size, ctx.style.heading_color, Weight::SEMIBOLD);
    ctx.active_block_marker = None;
}

fn layout_code_block(block: &BlockNode, ctx: &mut LayoutCtx, language: Option<&str>) {
    let is_mermaid = language.is_some_and(is_mermaid_language);
    let mut active = code_block_is_active(block, ctx.edit_ctx)
        || (is_mermaid && block_is_selected(block, ctx.selection_range));

    if is_mermaid && !active {
        let content = block.lines(ctx.doc).join("\n");
        if layout_embedded_block(
            block,
            ctx,
            crate::embedded::EmbeddedKind::Mermaid,
            content.trim_end(),
        ) {
            return;
        }
        active = true;
    }

    let font_size = ctx.style.code_font_size;
    let line_h = ctx.style.code_line_height;
    let pad = ctx.style.code_block_padding;

    let (lines, starts) = if active {
        let full_text = ctx.doc.doc_text_in_range(block.block_range.clone());
        let (lines, starts) = active_code_source_lines(
            &full_text,
            block.block_range.start,
            !is_fenced_code_block(block, ctx.doc),
        );
        (lines, Some(starts))
    } else if let Some(content_start) = empty_fenced_code_content_start(block, ctx.doc) {
        (vec![String::new()], Some(vec![content_start]))
    } else {
        let raw = collect_text_lines(block, ctx.doc);
        let lines_vec: Vec<String> =
            raw.iter().flat_map(|l| l.split('\n').map(|s| s.to_string())).collect();
        (lines_vec, block.code_line_source_starts.clone())
    };

    let ascii_diagram = if active || !is_fenced_code_block(block, ctx.doc) {
        None
    } else {
        super::ascii_diagram::detect_ascii_diagram(&lines)
    };

    let total_h = lines.len() as f32 * line_h + pad * 2.0;

    // Compute syntax highlight spans if a highlighter and language are available.
    let highlight_spans_per_line: Option<Vec<Vec<crate::builder::HighlightSpan>>> =
        if let (Some(hl), Some(lang)) = (ctx.highlighter, language) {
            let full_code = lines.join("\n");
            Some(hl.highlight(lang, &full_code))
        } else {
            None
        };

    let mut laid_out_lines = Vec::new();
    let mut ly = ctx.y + pad;
    for (i, line_text) in lines.iter().enumerate() {
        let hl_spans =
            highlight_spans_per_line.as_ref().and_then(|v| v.get(i).cloned()).unwrap_or_default();
        let source_projection = starts.as_ref().and_then(|line_starts| {
            line_starts.get(i).map(|&source_start| {
                crate::projection::ProjectedText::direct(line_text, source_start)
                    .slice_visual_line(0, 0..line_text.len())
                    .expect("a direct code-line projection must slice at its boundaries")
            })
        });
        let (shaped, _) = super::shaping::shape_line(
            line_text,
            font_size,
            Weight::NORMAL,
            shaping::Style::Normal,
            ctx.style.code_font_family.as_deref(),
            ctx.shaper.as_deref_mut(),
        );
        // Retain the code-font geometry so caret and hit-testing share render advances.
        laid_out_lines.push(LaidOutLine {
            text: line_text.clone(),
            rect: ui::core::geom::Rect::new(
                ctx.indent + pad,
                ly,
                ctx.available_width() - pad * 2.0,
                line_h,
            ),
            font_size,
            is_code: true,
            font_weight: Weight::NORMAL,
            color_override: None,
            doc_line_idx: i,
            styles: vec![],
            style_segments: vec![],
            shaped,
            text_layout: None,
            highlight_spans: hl_spans,
            source_projection,
        });
        ly += line_h;
    }

    if let Some(ascii_diagram) = ascii_diagram {
        ctx.ascii_diagrams.register(block.block_range.clone(), &laid_out_lines, ascii_diagram);
    }
    ctx.push_block(
        LaidOutBlockKind::CodeBlock {
            lines: laid_out_lines,
            language: language.map(str::to_owned),
        },
        total_h,
    );
}

fn layout_quote_block(block: &BlockNode, ctx: &mut LayoutCtx) {
    let saved_indent = ctx.indent;
    ctx.indent += ctx.style.blockquote_padding;
    let start_y = ctx.y;
    ctx.y += ctx.style.blockquote_padding; // top padding
    let saved_color_fade = ctx.color_fade;
    let saved_font_size_override = ctx.font_size_override;
    ctx.color_fade = 0.25; // blend text 25% toward bg
    ctx.font_size_override = Some(ctx.style.code_font_size);

    // Collect child blocks into a sub-layout
    let mut sub_blocks = Vec::new();
    let saved_output = std::mem::take(&mut ctx.output);
    // Detect active block marker: cursor in blockquote's source range.
    // The marker is active during child layout so that Paragraph children
    // in layout_text_block pick it up and prepend "> " to their first line.
    if let Some(edit_ctx) = ctx.edit_ctx {
        ctx.active_block_marker = crate::edit::active_block_marker(block, edit_ctx.cursor_byte);
    }
    let saved_previous_spacing_block = ctx.previous_spacing_block.take();
    let mut child_placement = BlockPlacement::AtContentOrigin;
    for child in &block.children {
        if layout_block_with_placement(child, ctx, child_placement) {
            child_placement = BlockPlacement::InFlow;
        }
    }
    ctx.previous_spacing_block = saved_previous_spacing_block;
    ctx.active_block_marker = None;
    sub_blocks.append(&mut ctx.output);
    ctx.output = saved_output;

    let content_h = ctx.y - start_y + ctx.style.blockquote_padding; // + bottom padding
    ctx.y = start_y;
    ctx.indent = saved_indent;
    ctx.color_fade = saved_color_fade;
    ctx.font_size_override = saved_font_size_override;

    ctx.push_block(LaidOutBlockKind::BlockQuote { blocks: sub_blocks }, content_h);
}

fn layout_list_item_block(
    block: &BlockNode,
    ctx: &mut LayoutCtx,
    bullet: &crate::builder::ListBullet,
) {
    let font_size = ctx.font_size_override.unwrap_or(ctx.style.body_font_size);
    let line_h = ctx.style.line_height;
    let saved_indent = ctx.indent;
    let saved_color_fade = ctx.color_fade;
    let saved_font_size_override = ctx.font_size_override;
    let current_depth = ctx.list_depth;
    let start_y = ctx.y;

    // Lay out item's own text as LaidOutLine (bullet offset)
    let mut item_lines = Vec::new();
    let raw_text_lines = list_item_text_lines_for_layout(block, ctx);
    // Detect active block marker: cursor in list item's source range.
    if let Some(edit_ctx) = ctx.edit_ctx {
        ctx.active_block_marker = if cursor_in_nested_list_item(block, edit_ctx.cursor_byte) {
            None
        } else {
            active_list_item_marker(block, ctx.source_text, edit_ctx.cursor_byte)
        };
    }
    for (line_idx, raw_line) in raw_text_lines.lines.iter().enumerate() {
        let line_styles = raw_line
            .styles
            .as_deref()
            .or_else(|| block.text_styles.get(line_idx).map(|s| s.as_slice()))
            .unwrap_or(&[]);
        let source_context = raw_line.source_context.as_ref();
        let mut projected = if let Some(source_text) = ctx.source_text {
            crate::edit::materialize_projected_line(
                &raw_line.projected,
                line_styles,
                source_text,
                ctx.edit_ctx,
                source_context,
            )
        } else {
            raw_line.projected.clone()
        };
        let mut materialized_styles = if let Some(source_text) = ctx.source_text {
            crate::edit::materialized_spans_for_projected_line(
                &raw_line.projected,
                &projected,
                line_styles,
                source_text,
                ctx.edit_ctx,
            )
        } else {
            line_styles.to_vec()
        };
        if line_idx == 0
            && let Some(marker) = ctx.active_block_marker.as_ref()
        {
            let marker_source_range = marker_source_range_for_projected_line(&projected, marker);
            projected = materialize_marker_for_projected_line(
                projected,
                marker,
                marker_source_range.clone(),
            );
            let marker_len = marker.marker_text.len();
            for style in &mut materialized_styles {
                style.start += marker_len;
            }
            materialized_styles.insert(
                0,
                StyleSpan {
                    start: 0,
                    len: marker_len,
                    style: crate::builder::InlineStyle::SourceMarker,
                    source_range: marker_source_range,
                },
            );
        }
        let active_ordered_marker = (line_idx == 0
            && matches!(bullet, crate::builder::ListBullet::Ordered(_)))
        .then_some(ctx.active_block_marker.as_ref())
        .flatten();
        let marker_len = active_ordered_marker.map(|marker| marker.marker_text.len());
        let estimated_marker_width = active_ordered_marker
            .map_or(0.0, |marker| marker.marker_text.chars().count() as f32 * font_size * 0.55);
        super::embedded::expose_failed_math(
            &mut projected,
            &mut materialized_styles,
            ctx.doc,
            font_size,
            ctx.style.text_color,
            !ctx.allow_embedded || ctx.shaper.is_none(),
            ctx.selection_range,
        );
        let projected_verbatim_ranges = projected_verbatim_ranges(&projected);
        let wrapped = ctx.wrap_text_with_first_line_indent_styled_and_protected(
            &projected.text,
            font_size,
            Weight::NORMAL,
            ctx.available_width(),
            0.0,
            &materialized_styles,
            &projected_verbatim_ranges,
            ctx.style.text_color,
        );
        let wrapped_shaped = ctx.last_wrap_shaped.first().cloned().flatten();
        let marker_width = marker_len
            .and_then(|marker_len| shaped_prefix_width(&ctx.last_wrap_shaped, marker_len))
            .unwrap_or(estimated_marker_width);
        let content_x = ctx.indent + ctx.style.list_indent;
        let x = (content_x - marker_width).max(ctx.indent);
        let reclaimed_width = content_x - x;
        let w = ctx.available_width() - ctx.style.list_indent + reclaimed_width;
        let laid = layout_line_with_styles(
            &materialized_styles,
            &wrapped,
            &projected,
            font_size,
            line_h,
            x,
            ctx.y,
            w,
            ctx.style.text_color,
            ctx.style.body_font_family.first().map(|s| s.as_str()),
            wrapped_shaped.as_ref(),
            line_idx,
            &ctx.embedded_images,
        );
        let n = laid.len();
        let laid_height = laid.iter().map(|line| line.rect.h).sum::<f32>();
        item_lines.extend(laid);
        ctx.y += if n == 0 { line_h } else { laid_height };
    }
    ctx.active_block_marker = None;

    // Lay out nested children with extra indent
    ctx.indent += ctx.style.list_indent;
    ctx.list_depth = current_depth + 1;
    let saved_output = std::mem::take(&mut ctx.output);
    let saved_previous_spacing_block = ctx.previous_spacing_block.take();
    let mut child_placement = BlockPlacement::AtContentOrigin;
    for (child_index, child) in block.children.iter().enumerate() {
        // 项文本与子块（loose list 的子段落）之间保留 paragraph_spacing
        // 级别的间距；嵌套子列表保持紧凑。与顶层块间间距同一视觉规范。
        if child_index == 0
            && !item_lines.is_empty()
            && let Some(child_spacing_block) = first_spacing_block(child)
        {
            ctx.y += super::spacing::list_item_child_gap(child_spacing_block, ctx.style);
        }
        if layout_block_with_placement(child, ctx, child_placement) {
            child_placement = BlockPlacement::InFlow;
        }
    }
    ctx.previous_spacing_block = saved_previous_spacing_block;
    let sub_blocks: Vec<LaidOutBlock> = ctx.output.drain(..).collect();
    ctx.output = saved_output;

    let content_h = (ctx.y - start_y).max(line_h);
    ctx.y = start_y;
    ctx.indent = saved_indent;
    ctx.color_fade = saved_color_fade;
    ctx.font_size_override = saved_font_size_override;
    ctx.list_depth = current_depth;

    ctx.push_block(
        LaidOutBlockKind::ListItem {
            bullet: bullet.clone(),
            blocks: sub_blocks,
            lines: item_lines,
            level_indent: ctx.style.list_indent,
            depth: current_depth,
        },
        content_h,
    );
}

fn layout_horizontal_rule(block: &BlockNode, ctx: &mut LayoutCtx) {
    let active = if let Some(edit_ctx) = ctx.edit_ctx {
        let rule_source = ctx.doc.doc_text_in_range(block.block_range.clone());
        let marker_end = block.block_range.start + rule_source.trim_end_matches(['\r', '\n']).len();
        edit_ctx.cursor_byte >= block.block_range.start && edit_ctx.cursor_byte <= marker_end
    } else {
        false
    };
    if active {
        let font_size = ctx.font_size_override.unwrap_or(ctx.style.code_font_size);
        let color = crate::style::blend_toward_bg(
            ctx.style.text_color,
            ctx.style.background_color,
            0.55, // SOURCE_MARKER_FADE_RATIO
        );
        layout_text_block(block, ctx, font_size, color, Weight::NORMAL);
    } else {
        ctx.push_block(LaidOutBlockKind::HorizontalRule, ctx.style.rule_thickness);
    }
}

fn layout_metadata_block(block: &BlockNode, ctx: &mut LayoutCtx) {
    let font_size = ctx.style.code_font_size;
    let line_h = ctx.style.code_line_height;
    let pad = ctx.style.code_block_padding;
    let lines = metadata_lines_with_source_starts(block);
    let total_h = lines.len() as f32 * line_h + pad * 2.0;

    let mut laid_out_lines = Vec::new();
    let mut ly = ctx.y + pad;
    for (line_idx, (line_text, source_start)) in lines.iter().enumerate() {
        let source_projection = crate::projection::ProjectedText::direct(line_text, *source_start)
            .slice_visual_line(0, 0..line_text.len())
            .expect("a direct metadata-line projection must slice at its boundaries");
        laid_out_lines.push(LaidOutLine {
            text: line_text.clone(),
            rect: ui::core::geom::Rect::new(
                ctx.indent + pad,
                ly,
                ctx.available_width() - pad * 2.0,
                line_h,
            ),
            font_size,
            is_code: true,
            font_weight: Weight::NORMAL,
            color_override: Some(ctx.style.code_color),
            doc_line_idx: line_idx,
            styles: vec![],
            style_segments: vec![],
            shaped: None,
            text_layout: None,
            highlight_spans: vec![],
            source_projection: Some(source_projection),
        });
        ly += line_h;
    }

    ctx.push_block(LaidOutBlockKind::MetadataBlock { lines: laid_out_lines }, total_h);
}

pub(in crate::layout::block) fn heading_marker_at_cursor(
    block: &BlockNode,
    source: &str,
    cursor_byte: usize,
) -> Option<crate::edit::ActiveBlockMarker> {
    if cursor_byte < block.block_range.start
        || cursor_byte > block.block_range.end
        || !crate::augmenter::heading_source_is_atx(source, block.block_range.start)
    {
        return None;
    }
    let spelling = &source[block.block_range.clone()];
    let line = spelling.split('\n').next()?.trim_end_matches('\r');
    let indentation = line.len() - line.trim_start_matches(' ').len();
    let hashes = line[indentation..].bytes().take_while(|&byte| byte == b'#').count();
    let hash_end = indentation + hashes;
    let separator =
        line[hash_end..].bytes().take_while(|byte| matches!(byte, b' ' | b'\t')).count();
    let prefix_end = block.block_range.start + hash_end + separator;
    if separator > 0 && cursor_byte >= prefix_end {
        return None;
    }
    Some(crate::edit::ActiveBlockMarker {
        marker_text: line[..hash_end + separator].to_owned(),
        marker_source_range: block.block_range.start..prefix_end,
    })
}

pub(in crate::layout::block) fn first_spacing_block(
    block: &BlockNode,
) -> Option<super::spacing::BlockSpacing> {
    super::spacing::BlockSpacing::from_block_kind(&block.kind)
        .or_else(|| block.children.iter().find_map(first_spacing_block))
}

pub(in crate::layout::block) fn shaped_prefix_width(
    shaped_lines: &[Option<shaping::ShapedRun>],
    prefix_byte_end: usize,
) -> Option<f32> {
    let shaped = shaped_lines.first()?.as_ref()?;
    Some(
        shaped
            .clusters
            .iter()
            .take_while(|cluster| cluster.byte_range.start < prefix_byte_end)
            .map(|cluster| cluster.advance)
            .sum(),
    )
}

pub(in crate::layout::block) fn active_list_item_marker(
    block: &BlockNode,
    source: Option<&str>,
    cursor_byte: usize,
) -> Option<crate::edit::ActiveBlockMarker> {
    let computed_marker = crate::edit::active_block_marker(block, cursor_byte)?;
    if !matches!(
        block.kind,
        BlockKind::ListItem { bullet: crate::builder::ListBullet::Ordered(_), .. }
    ) {
        return Some(computed_marker);
    }

    source.and_then(|source| ordered_source_marker(block, source)).or(Some(computed_marker))
}

pub(in crate::layout::block) fn ordered_source_marker(
    block: &BlockNode,
    source: &str,
) -> Option<crate::edit::ActiveBlockMarker> {
    let mut marker_start = block.block_range.start;
    while matches!(source.as_bytes().get(marker_start), Some(b' ' | b'\t')) {
        marker_start += 1;
    }
    let (bullet, content_start) = crate::augmenter::parse_list_marker(source, marker_start)?;
    if !matches!(bullet, crate::builder::ListBullet::Ordered(_)) {
        return None;
    }

    Some(crate::edit::ActiveBlockMarker {
        marker_text: source.get(marker_start..content_start)?.to_owned(),
        marker_source_range: marker_start..content_start,
    })
}

pub(crate) fn is_mermaid_language(language_hint: &str) -> bool {
    cfg!(feature = "rich-markdown")
        && language_hint
            .split_whitespace()
            .next()
            .is_some_and(|language| language.eq_ignore_ascii_case("mermaid"))
}

pub(in crate::layout::block) fn block_is_selected(
    block: &BlockNode,
    selection: Option<&std::ops::Range<usize>>,
) -> bool {
    selection.is_some_and(|range| {
        range.start < block.block_range.end && block.block_range.start < range.end
    })
}

pub(in crate::layout::block) fn block_is_active(block: &BlockNode, ctx: &LayoutCtx<'_>) -> bool {
    code_block_is_active(block, ctx.edit_ctx) || block_is_selected(block, ctx.selection_range)
}

pub(in crate::layout::block) fn layout_embedded_block(
    block: &BlockNode,
    ctx: &mut LayoutCtx<'_>,
    kind: crate::embedded::EmbeddedKind,
    source: &str,
) -> bool {
    if !ctx.allow_embedded {
        return false;
    }
    let foreground =
        ctx.style.text_color.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
    let request = crate::embedded::EmbeddedRequest {
        kind,
        source,
        font_size: ctx.style.body_font_size,
        foreground,
        background: [0, 0, 0, 0],
    };
    let Ok(image) = crate::embedded::render_embedded(request) else {
        return false;
    };
    let scale = (ctx.available_width() / image.image.width() as f32).min(1.0);
    let height = image.image.height() as f32 * scale;
    ctx.embedded_images.register(block.block_range.clone(), image.image, image.baseline * scale);
    ctx.push_block(LaidOutBlockKind::Embedded { source_range: block.block_range.clone() }, height);
    true
}

pub(in crate::layout::block) fn code_block_is_active(
    block: &BlockNode,
    edit_ctx: Option<&crate::edit::EditContext>,
) -> bool {
    code_block_cursor_is_inside(block, edit_ctx)
}

const INDENTED_CODE_COLUMNS: usize = 4;

pub(in crate::layout::block) fn active_code_source_lines(
    source: &str,
    source_start: usize,
    indented: bool,
) -> (Vec<String>, Vec<usize>) {
    let mut lines = Vec::new();
    let mut starts = Vec::new();
    let mut line_start = source_start;

    for (line_index, raw_line) in source.split('\n').enumerate() {
        let line = raw_line.trim_end_matches('\r');
        let indentation_bytes =
            if indented && line_index > 0 { indented_code_content_start(line) } else { 0 };
        lines.push(line[indentation_bytes..].to_owned());
        starts.push(line_start + indentation_bytes);
        line_start += raw_line.len() + 1;
    }

    (lines, starts)
}

pub(in crate::layout::block) fn indented_code_content_start(line: &str) -> usize {
    if line.trim_matches([' ', '\t']).is_empty() {
        return line.len();
    }

    let mut columns = 0;
    for (byte, character) in line.char_indices() {
        let width = match character {
            ' ' => 1,
            '\t' => INDENTED_CODE_COLUMNS - columns % INDENTED_CODE_COLUMNS,
            _ => return byte,
        };
        columns += width;
        if columns >= INDENTED_CODE_COLUMNS {
            return byte + character.len_utf8();
        }
    }

    line.len()
}

pub(in crate::layout::block) fn code_block_cursor_is_inside(
    block: &BlockNode,
    edit_ctx: Option<&crate::edit::EditContext>,
) -> bool {
    edit_ctx.is_some_and(|edit_ctx| {
        block.block_range.start <= edit_ctx.cursor_byte
            && edit_ctx.cursor_byte <= block.block_range.end
    })
}

pub(in crate::layout::block) fn is_fenced_code_block(
    block: &BlockNode,
    doc: &dyn textora_core::document::DocView,
) -> bool {
    let block_text = doc.doc_text_in_range(block.block_range.clone());
    let mut first_line = block_text.lines().next().unwrap_or_default();
    for _ in 0..3 {
        let Some(without_indent) = first_line.strip_prefix(' ') else {
            break;
        };
        first_line = without_indent;
    }

    let Some(fence_character) = first_line.chars().next() else {
        return false;
    };
    if !matches!(fence_character, '`' | '~') {
        return false;
    }

    first_line.chars().take_while(|character| *character == fence_character).count() >= 3
}

pub(in crate::layout::block) fn empty_fenced_code_content_start(
    block: &BlockNode,
    doc: &dyn textora_core::document::DocView,
) -> Option<usize> {
    if !block.text_lines.is_empty()
        || block.code_line_source_starts.is_some()
        || !is_fenced_code_block(block, doc)
    {
        return None;
    }

    let source = doc.doc_text_in_range(block.block_range.clone());
    let opening_line_end = source.find('\n')?;
    Some(block.block_range.start + opening_line_end + '\n'.len_utf8())
}

pub(in crate::layout::block) fn metadata_lines_with_source_starts(
    block: &BlockNode,
) -> Vec<(String, usize)> {
    block.projected_lines.iter().flat_map(metadata_projected_line_starts).collect()
}

pub(in crate::layout::block) fn metadata_projected_line_starts(
    projected: &crate::projection::ProjectedText,
) -> Vec<(String, usize)> {
    let mut lines = Vec::new();
    let mut visual_byte_start = 0usize;

    for line_text in projected.text.split('\n') {
        let grapheme_start =
            crate::grapheme_map::grapheme_index_at_byte(&projected.text, visual_byte_start);
        let source_start = projected
            .boundaries
            .get(grapheme_start)
            .expect("a projected metadata line must have a source boundary")
            .byte
            .get();
        lines.push((line_text.to_string(), source_start));
        visual_byte_start += line_text.len() + 1;
    }

    lines
}

#[path = "block_table.rs"]
mod block_table;
#[path = "block_text.rs"]
mod block_text;

#[allow(unused_imports)]
pub(in crate::layout::block) use block_table::*;
#[allow(unused_imports)]
pub(in crate::layout::block) use block_text::*;

#[cfg(test)]
#[path = "block_tests.rs"]
mod tests;
