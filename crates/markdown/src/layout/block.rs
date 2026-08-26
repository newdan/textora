//! Block-level layout functions.

use shaping::{Shaper, Weight};

use super::context::LayoutCtx;
use super::heading_spacing_scale;
use super::types::{LaidOutBlock, LaidOutBlockKind, LaidOutLine};
use crate::builder::{BlockKind, BlockNode, BlockSource, StyleSpan};
use crate::edit::SourceLineContext;
use crate::style::MarkdownStyle;

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
    text_doc: &dyn core::document::DocView,
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
    text_doc: &dyn core::document::DocView,
) -> LaidOutDoc {
    layout_doc_with_shaper_for_rendering(blocks, style, viewport_w, shaper, highlighter, text_doc)
        .doc
}

/// A Markdown layout that retains crate-private render sidecars.
///
/// Its [`LaidOutDoc`] remains available through [`Self::document`], while
/// Markdown-specific metadata stays encapsulated so UI consumers do not need
/// to name or depend on Markdown grid types.
pub struct MarkdownLayout {
    pub(crate) doc: LaidOutDoc,
    pub(crate) ascii_diagrams: super::ascii_diagram::AsciiDiagramRegistry,
}

impl MarkdownLayout {
    /// Returns the positioned document for consumers that only need layout data.
    pub fn document(&self) -> &LaidOutDoc {
        &self.doc
    }

    pub(crate) fn ascii_diagrams(&self) -> &super::ascii_diagram::AsciiDiagramRegistry {
        &self.ascii_diagrams
    }
}

/// Layout a document for rendering while retaining Markdown render sidecars.
pub fn layout_doc_for_rendering(
    blocks: &[BlockNode],
    style: &MarkdownStyle,
    viewport_w: f32,
    text_doc: &dyn core::document::DocView,
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
    text_doc: &dyn core::document::DocView,
) -> MarkdownLayout {
    let mut ctx = LayoutCtx::new(text_doc, style, viewport_w, shaper, highlighter, None, None);

    for block in blocks {
        layout_block(block, &mut ctx);
    }

    MarkdownLayout {
        doc: super::types::LaidOutDoc { blocks: ctx.output, total_height: ctx.y },
        ascii_diagrams: ctx.ascii_diagrams,
    }
}

use super::types::LaidOutDoc;

pub(crate) fn layout_block(block: &BlockNode, ctx: &mut LayoutCtx) {
    // End of a list group: the ListItem handler already added list_item_spacing
    // as trailing for each item. Bump the gap to list_group_spacing so the
    // transition from list to non-list feels like a normal block boundary.
    if ctx.last_block_was_list && !matches!(block.kind, BlockKind::ListItem { .. }) {
        ctx.y += ctx.style.list_group_spacing - ctx.style.list_item_spacing;
        ctx.last_trailing_spacing = ctx.style.list_group_spacing;
    }
    // Save list flag before reset; ListItem handler needs the previous value.
    // Note: last_block_was_heading is NOT reset here — it's managed by
    // Heading handler for margin collapsing between adjacent headings.
    ctx.last_block_was_list = false;
    match &block.kind {
        BlockKind::Container => {
            for child in &block.children {
                layout_block(child, ctx);
            }
        }
        BlockKind::Paragraph => {
            let font_size = ctx.font_size_override.unwrap_or(ctx.style.body_font_size);
            layout_text_block(block, ctx, font_size, ctx.style.text_color, Weight::NORMAL);
            ctx.last_block_was_heading = false;
            ctx.last_block_was_list = false;
            ctx.block_count += 1;
            ctx.last_block_kind = Some(super::context::LastBlockKind::Paragraph);
            ctx.y += ctx.style.paragraph_spacing;
            ctx.last_trailing_spacing = ctx.style.paragraph_spacing;
        }
        BlockKind::Heading { level } => {
            let idx = (*level as usize).saturating_sub(1).min(5);
            let font_size = ctx.style.heading_font_sizes[idx];
            // Heading top spacing: scale by level, collapse with previous trailing.
            // H1 keeps full, H2-H3 80%, H4-H6 65%.
            let level_scale = heading_spacing_scale(*level);
            if !ctx.last_block_was_heading {
                let desired_top = ctx.style.heading_spacing_top * level_scale;
                let extra = if ctx.block_count == 0 {
                    // First block: halve the top spacing
                    desired_top * 0.5
                } else {
                    // Margin collapsing: only add the excess over previous trailing
                    (desired_top - ctx.last_trailing_spacing).max(0.0)
                };
                ctx.y += extra;
            }
            // Detect active block marker: cursor in heading's source range.
            if let Some(edit_ctx) = ctx.edit_ctx {
                ctx.active_block_marker =
                    crate::edit::active_block_marker(block, edit_ctx.cursor_byte);
            }
            layout_text_block(block, ctx, font_size, ctx.style.heading_color, Weight::SEMIBOLD);
            ctx.active_block_marker = None;
            ctx.y += ctx.style.heading_spacing_bottom;
            ctx.last_block_was_heading = true;
            ctx.last_block_was_list = false;
            ctx.last_block_kind = Some(super::context::LastBlockKind::Heading);
            ctx.last_trailing_spacing = ctx.style.heading_spacing_bottom;
            ctx.block_count += 1;
        }
        BlockKind::CodeBlock { language } => {
            let active = code_block_is_active(block, ctx.edit_ctx);

            let font_size = ctx.style.code_font_size;
            let line_h = ctx.style.code_line_height;
            let pad = ctx.style.code_block_padding;

            let (lines, starts) = if active {
                let full_text = ctx.doc.doc_text_in_range(block.block_range.clone());
                let lines_vec: Vec<String> = full_text.split('\n').map(|s| s.to_string()).collect();
                let mut current = block.block_range.start;
                let mut st = Vec::new();
                for l in &lines_vec {
                    st.push(current);
                    current += l.len() + 1; // +1 for \n
                }
                (lines_vec, Some(st))
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
                if let (Some(hl), Some(lang)) = (ctx.highlighter, language.as_deref()) {
                    let full_code = lines.join("\n");
                    Some(hl.highlight(lang, &full_code))
                } else {
                    None
                };

            let mut laid_out_lines = Vec::new();
            let mut ly = ctx.y + pad;
            for (i, line_text) in lines.iter().enumerate() {
                let hl_spans = highlight_spans_per_line
                    .as_ref()
                    .and_then(|v| v.get(i).cloned())
                    .unwrap_or_default();
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
                ctx.ascii_diagrams.register(
                    block.block_range.clone(),
                    &laid_out_lines,
                    ascii_diagram,
                );
            }
            ctx.push_block(
                LaidOutBlockKind::CodeBlock { lines: laid_out_lines, language: language.clone() },
                total_h,
            );
            ctx.last_block_was_heading = false;
            ctx.last_block_was_list = false;
            ctx.last_block_kind = Some(super::context::LastBlockKind::CodeBlock);
            ctx.block_count += 1;
            ctx.y += ctx.style.paragraph_spacing;
            ctx.last_trailing_spacing = ctx.style.paragraph_spacing;
        }
        BlockKind::BlockQuote => {
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
                ctx.active_block_marker =
                    crate::edit::active_block_marker(block, edit_ctx.cursor_byte);
            }
            for child in &block.children {
                layout_block(child, ctx);
            }
            ctx.active_block_marker = None;
            sub_blocks.append(&mut ctx.output);
            ctx.output = saved_output;

            // Remove trailing spacing from the last child so blockquote height
            // doesn't include inter-block spacing that belongs to the parent context.
            if let Some(last_child) = block.children.last() {
                match last_child.kind {
                    BlockKind::Paragraph => ctx.y -= ctx.style.paragraph_spacing,
                    BlockKind::Heading { .. } => ctx.y -= ctx.style.heading_spacing_bottom,
                    _ => {}
                }
            }

            let content_h = ctx.y - start_y + ctx.style.blockquote_padding; // + bottom padding
            ctx.y = start_y;
            ctx.indent = saved_indent;
            ctx.color_fade = saved_color_fade;
            ctx.font_size_override = saved_font_size_override;

            ctx.push_block(LaidOutBlockKind::BlockQuote { blocks: sub_blocks }, content_h);
            ctx.y += ctx.style.paragraph_spacing; // spacing after blockquote
            ctx.last_block_was_heading = false;
            ctx.last_block_was_list = false;
            ctx.block_count += 1;
            ctx.last_block_kind = Some(super::context::LastBlockKind::BlockQuote);
            ctx.last_trailing_spacing = ctx.style.paragraph_spacing;
        }
        BlockKind::ListItem { bullet, tight, blank_line_before } => {
            // For tight lists immediately following a paragraph (no blank line),
            // reduce the gap from paragraph_spacing to list_item_spacing.
            if *tight
                && !*blank_line_before
                && !ctx.last_block_was_list
                && ctx.last_block_kind == Some(super::context::LastBlockKind::Paragraph)
                && ctx.last_trailing_spacing > ctx.style.list_item_spacing
            {
                let reduce = ctx.last_trailing_spacing - ctx.style.list_item_spacing;
                ctx.y -= reduce;
                ctx.last_trailing_spacing = ctx.style.list_item_spacing;
            }
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
                ctx.active_block_marker = if cursor_in_nested_list_item(block, edit_ctx.cursor_byte)
                {
                    None
                } else {
                    crate::edit::active_block_marker(block, edit_ctx.cursor_byte)
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
                let materialized_spans = if let Some(source_text) = ctx.source_text {
                    crate::edit::materialized_spans_for_projected_line(
                        &raw_line.projected,
                        line_styles,
                        source_text,
                        ctx.edit_ctx,
                    )
                } else {
                    crate::edit::materialize_line(&raw_line.projected.text, line_styles, "", None)
                        .spans
                };
                let mut materialized_styles: Vec<StyleSpan> = materialized_spans
                    .iter()
                    .map(|span| StyleSpan {
                        start: span.start,
                        len: span.len,
                        style: span.style.clone(),
                        source_range: span.source_range.clone(),
                    })
                    .collect();
                if line_idx == 0
                    && let Some(marker) = ctx.active_block_marker.as_ref()
                {
                    let marker_source_range =
                        marker_source_range_for_projected_line(&projected, marker);
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
                let wrapped = ctx.wrap_text(&projected.text, font_size, Weight::NORMAL);
                let x = ctx.indent + ctx.style.list_indent;
                let w = ctx.available_width() - ctx.style.list_indent;
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
                    ctx.shaper.as_deref_mut(),
                    line_idx,
                );
                let n = laid.len();
                item_lines.extend(laid);
                ctx.y += line_h * n as f32;
            }
            ctx.active_block_marker = None;

            // Lay out nested children with extra indent
            ctx.indent += ctx.style.list_indent;
            ctx.list_depth = current_depth + 1;
            let saved_output = std::mem::take(&mut ctx.output);
            for child in &block.children {
                layout_block(child, ctx);
            }
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
            // Uniform inter-item spacing: added after every list item,
            // so all items have the same rect.h (content-only, no spacing baked in).
            ctx.y += ctx.style.list_item_spacing;
            ctx.last_block_was_heading = false;
            ctx.last_block_was_list = true;
            ctx.block_count += 1;
            ctx.last_block_kind = Some(super::context::LastBlockKind::ListItem);
            ctx.last_trailing_spacing = ctx.style.list_item_spacing;
        }
        BlockKind::TableWrapper { columns, alignments: _ } => {
            layout_table(block, ctx, *columns);
            ctx.last_block_was_heading = false;
            ctx.last_block_was_list = false;
            ctx.block_count += 1;
            ctx.last_block_kind = Some(super::context::LastBlockKind::TableWrapper);
            ctx.y += ctx.style.paragraph_spacing;
            ctx.last_trailing_spacing = ctx.style.paragraph_spacing;
        }
        BlockKind::TableRow_ | BlockKind::TableCell_ { .. } => {
            for child in &block.children {
                layout_block(child, ctx);
            }
        }
        BlockKind::HorizontalRule => {
            let active = if let Some(edit_ctx) = ctx.edit_ctx {
                edit_ctx.cursor_byte >= block.block_range.start
                    && edit_ctx.cursor_byte <= block.block_range.end
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
                ctx.last_block_was_heading = false;
                ctx.last_block_was_list = false;
                ctx.block_count += 1;
                ctx.last_block_kind = Some(super::context::LastBlockKind::Paragraph);
                ctx.y += ctx.style.paragraph_spacing;
                ctx.last_trailing_spacing = ctx.style.paragraph_spacing;
            } else {
                ctx.push_block(
                    LaidOutBlockKind::HorizontalRule,
                    ctx.style.rule_spacing + ctx.style.rule_thickness + ctx.style.rule_spacing,
                );
                ctx.last_block_was_heading = false;
                ctx.last_block_was_list = false;
                ctx.block_count += 1;
                ctx.last_block_kind = Some(super::context::LastBlockKind::HorizontalRule);
                ctx.last_trailing_spacing = ctx.style.rule_spacing;
            }
        }
        BlockKind::MetadataBlock => {
            let font_size = ctx.style.code_font_size;
            let line_h = ctx.style.code_line_height;
            let pad = ctx.style.code_block_padding;
            let lines = metadata_lines_with_source_starts(block);
            let total_h = lines.len() as f32 * line_h + pad * 2.0;

            let mut laid_out_lines = Vec::new();
            let mut ly = ctx.y + pad;
            for (line_idx, (line_text, source_start)) in lines.iter().enumerate() {
                let source_projection =
                    crate::projection::ProjectedText::direct(line_text, *source_start)
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
            ctx.last_block_was_heading = false;
            ctx.last_block_was_list = false;
            ctx.block_count += 1;
            ctx.last_block_kind = Some(super::context::LastBlockKind::MetadataBlock);
            ctx.y += ctx.style.paragraph_spacing;
            ctx.last_trailing_spacing = ctx.style.paragraph_spacing;
        }
    }
}

fn code_block_is_active(block: &BlockNode, edit_ctx: Option<&crate::edit::EditContext>) -> bool {
    code_block_cursor_is_inside(block, edit_ctx)
}

fn code_block_cursor_is_inside(
    block: &BlockNode,
    edit_ctx: Option<&crate::edit::EditContext>,
) -> bool {
    edit_ctx.is_some_and(|edit_ctx| {
        block.block_range.start <= edit_ctx.cursor_byte
            && edit_ctx.cursor_byte <= block.block_range.end
    })
}

fn is_fenced_code_block(block: &BlockNode, doc: &dyn core::document::DocView) -> bool {
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

fn empty_fenced_code_content_start(
    block: &BlockNode,
    doc: &dyn core::document::DocView,
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

fn metadata_lines_with_source_starts(block: &BlockNode) -> Vec<(String, usize)> {
    block.projected_lines.iter().flat_map(metadata_projected_line_starts).collect()
}

fn metadata_projected_line_starts(
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
            .byte;
        lines.push((line_text.to_string(), source_start));
        visual_byte_start += line_text.len() + 1;
    }

    lines
}

struct LayoutTextLine {
    projected: crate::projection::ProjectedText,
    styles: Option<Vec<StyleSpan>>,
    source_context: Option<SourceLineContext>,
}

struct ListItemTextLines {
    lines: Vec<LayoutTextLine>,
}

fn list_item_text_lines_for_layout(block: &BlockNode, ctx: &LayoutCtx<'_>) -> ListItemTextLines {
    if !block.children.is_empty() && block.text_lines.is_empty() && block.text_styles.is_empty() {
        return ListItemTextLines { lines: Vec::new() };
    }

    ListItemTextLines { lines: list_item_projected_lines(block, ctx.doc) }
}

fn cursor_in_nested_list_item(block: &BlockNode, cursor_byte: usize) -> bool {
    block.children.iter().any(|child| {
        let cursor_in_child =
            child.block_range.start <= cursor_byte && cursor_byte <= child.block_range.end;
        let child_is_list_item = matches!(child.kind, BlockKind::ListItem { .. });
        (child_is_list_item && cursor_in_child) || cursor_in_nested_list_item(child, cursor_byte)
    })
}

fn source_line_context_for_layout(
    block: &BlockNode,
    doc: &dyn core::document::DocView,
    line_idx: usize,
    line_text: &str,
) -> Option<SourceLineContext> {
    match &block.source_range {
        BlockSource::Fragmented(ranges) => ranges
            .get(line_idx)
            .map(|range| SourceLineContext { range: range.clone(), text_start: range.start }),
        BlockSource::Continuous(range) => {
            let mut line_start = range.start;
            for (idx, current_line) in block.lines(doc).iter().enumerate() {
                let current_len = source_line_len(block, idx, current_line);
                if idx == line_idx {
                    let marker_len =
                        if idx == 0 { crate::edit::block_marker_len(block) } else { 0 };
                    return Some(SourceLineContext {
                        range: line_start..line_start + current_len,
                        text_start: line_start + marker_len,
                    });
                }
                line_start += current_len + 1;
            }

            Some(SourceLineContext {
                range: line_start..line_start + line_text.len(),
                text_start: line_start,
            })
        }
    }
}

fn source_line_len(block: &BlockNode, line_idx: usize, line_text: &str) -> usize {
    let content_len = if let Some(spans) = block.text_styles.get(line_idx)
        && !spans.is_empty()
    {
        let mut source_len = 0usize;
        let mut folded_pos = 0usize;
        for span in spans {
            source_len += span.start - folded_pos;
            source_len += span.source_range.end - span.source_range.start;
            folded_pos = span.start + span.len;
        }
        source_len + line_text.len() - folded_pos
    } else {
        line_text.len()
    };

    if line_idx == 0 { content_len + crate::edit::block_marker_len(block) } else { content_len }
}

fn list_item_projected_lines(
    block: &BlockNode,
    doc: &dyn core::document::DocView,
) -> Vec<LayoutTextLine> {
    let projected_lines = if block.projected_lines.is_empty() {
        block
            .lines(doc)
            .into_iter()
            .enumerate()
            .map(|(line_idx, line)| {
                let source_context = source_line_context_for_layout(block, doc, line_idx, &line);
                crate::projection::ProjectedText::direct(
                    &line,
                    source_context.as_ref().map_or(0, |context| context.text_start),
                )
            })
            .collect()
    } else {
        block.projected_lines.clone()
    };

    projected_lines
        .into_iter()
        .enumerate()
        .map(|(line_idx, projected)| {
            let source_extent = projected.source_extent();
            let visual_range = 0..projected.text.len();
            LayoutTextLine {
                styles: styles_for_projected_visual_range(
                    block.text_styles.get(line_idx).map(Vec::as_slice).unwrap_or(&[]),
                    visual_range,
                    &projected,
                ),
                source_context: Some(SourceLineContext {
                    text_start: source_extent.start,
                    range: source_extent,
                }),
                projected,
            }
        })
        .collect()
}

fn styles_for_projected_visual_range(
    styles: &[StyleSpan],
    visual_range: std::ops::Range<usize>,
    projected: &crate::projection::ProjectedText,
) -> Option<Vec<StyleSpan>> {
    (!styles.is_empty()).then(|| {
        styles
            .iter()
            .filter_map(|span| {
                let span_end = span.start + span.len;
                let clipped_start = span.start.max(visual_range.start);
                let clipped_end = span_end.min(visual_range.end);
                (clipped_start < clipped_end).then(|| StyleSpan {
                    start: clipped_start - visual_range.start,
                    len: clipped_end - clipped_start,
                    style: span.style.clone(),
                    source_range: projected_style_source_range(
                        projected,
                        span,
                        clipped_start..clipped_end,
                    ),
                })
            })
            .collect()
    })
}

fn projected_style_source_range(
    projected: &crate::projection::ProjectedText,
    style_span: &StyleSpan,
    visual_range: std::ops::Range<usize>,
) -> std::ops::Range<usize> {
    let style_end = style_span.start + style_span.len;
    let source_start = if visual_range.start == style_span.start {
        style_span.source_range.start
    } else {
        projected.boundaries
            [crate::grapheme_map::grapheme_index_at_byte(&projected.text, visual_range.start)]
        .byte
    };
    let source_end = if visual_range.end == style_end {
        style_span.source_range.end
    } else {
        projected.boundaries
            [crate::grapheme_map::grapheme_index_at_byte(&projected.text, visual_range.end)]
        .byte
    };
    source_start..source_end
}

fn marker_source_range_for_projected_line(
    projected: &crate::projection::ProjectedText,
    marker: &crate::edit::ActiveBlockMarker,
) -> std::ops::Range<usize> {
    let content_start =
        projected.boundaries.first().expect("a projected line always has a sentinel boundary").byte;
    let marker_len = marker.marker_text.len();
    content_start.saturating_sub(marker_len)..content_start
}

fn materialize_marker_for_projected_line(
    projected: crate::projection::ProjectedText,
    marker: &crate::edit::ActiveBlockMarker,
    marker_source_range: std::ops::Range<usize>,
) -> crate::projection::ProjectedText {
    crate::edit::materialize_block_marker(
        projected,
        &crate::edit::ActiveBlockMarker {
            marker_text: marker.marker_text.clone(),
            marker_source_range,
        },
    )
}

pub(crate) fn estimate_line_count(text: &str, max_w: f32, font_size: f32) -> usize {
    estimated_visual_line_ranges(text, max_w, font_size).len()
}

fn estimated_visual_line_ranges(
    text: &str,
    max_w: f32,
    font_size: f32,
) -> Vec<std::ops::Range<usize>> {
    if text.is_empty() {
        let empty_text_range = 0..0;
        return vec![empty_text_range];
    }
    let char_w = font_size * 0.55;
    let graphemes_per_line = (max_w / char_w).max(1.0) as usize;
    let visual_grapheme_bytes = crate::grapheme_map::grapheme_byte_boundaries(text);
    let grapheme_count = visual_grapheme_bytes.len().saturating_sub(1);
    let mut ranges = Vec::with_capacity(grapheme_count.div_ceil(graphemes_per_line));
    for grapheme_start in (0..grapheme_count).step_by(graphemes_per_line) {
        let grapheme_end = (grapheme_start + graphemes_per_line).min(grapheme_count);
        ranges.push(visual_grapheme_bytes[grapheme_start]..visual_grapheme_bytes[grapheme_end]);
    }
    ranges
}

pub(crate) fn layout_text_block(
    block: &BlockNode,
    ctx: &mut LayoutCtx,
    font_size: f32,
    color: [f32; 4],
    font_weight: Weight,
) {
    let line_h = if font_size >= ctx.style.heading_font_sizes[1] {
        font_size * 1.3
    } else {
        ctx.style.line_height
    };

    let raw_lines = collect_text_lines(block, ctx.doc);
    let raw_styles = &block.text_styles;
    let mut laid_out_lines = Vec::new();
    let mut ly = ctx.y;

    for (line_idx, raw) in raw_lines.iter().enumerate() {
        if ctx.shaper.is_none() {
            // Estimate line count from raw text only — the active block marker
            // must NOT participate, or the count would be wrong for content
            // near the wrap boundary. The marker is prepended downstream by
            // the precision path (shaper available), which handles wrapping
            // and source-map adjustment via prepend_marker_to_line.
            let line_styles = raw_styles.get(line_idx).cloned().unwrap_or_default();
            let source_projection =
                block.projected_lines.get(line_idx).cloned().unwrap_or_else(|| {
                    crate::projection::ProjectedText::direct(raw, block.block_range.start)
                });
            let estimated_ranges = estimated_visual_line_ranges(
                &source_projection.text,
                ctx.available_width(),
                font_size,
            );
            let visual_grapheme_bytes =
                crate::grapheme_map::grapheme_byte_boundaries(&source_projection.text);
            for (i, visual_range) in estimated_ranges.into_iter().enumerate() {
                let text = if i == 0 { raw.to_string() } else { String::new() };
                laid_out_lines.push(LaidOutLine {
                    // Put full raw text and styles in the first line so flat_lines
                    // have content. Wrapped lines (i>0) get empty — precision fills them.
                    text,
                    styles: if i == 0 { line_styles.clone() } else { vec![] },
                    rect: ui::core::geom::Rect::new(ctx.indent, ly, ctx.available_width(), line_h),
                    font_size,
                    is_code: false,
                    font_weight,
                    color_override: Some(if ctx.color_fade > 0.0 {
                        crate::style::blend_toward_bg(
                            color,
                            ctx.style.background_color,
                            ctx.color_fade,
                        )
                    } else {
                        color
                    }),
                    doc_line_idx: line_idx,
                    style_segments: vec![],
                    shaped: None,
                    text_layout: None,
                    highlight_spans: vec![],
                    source_projection: Some(
                        source_projection
                            .slice_visual_line_indexed(&visual_grapheme_bytes, i, visual_range)
                            .expect("estimated wrapped lines must preserve source projections"),
                    ),
                });
                ly += line_h;
            }
            continue;
        }

        let line_styles = raw_styles.get(line_idx).map(|s| s.as_slice()).unwrap_or(&[]);
        let source_context = source_line_context_for_layout(block, ctx.doc, line_idx, raw);
        let base_projection = block.projected_lines.get(line_idx).cloned().unwrap_or_else(|| {
            crate::projection::ProjectedText::direct(
                raw,
                source_context.as_ref().map_or(0, |context| context.text_start),
            )
        });
        let mut projected = if let Some(source_text) = ctx.source_text {
            crate::edit::materialize_projected_line(
                &base_projection,
                line_styles,
                source_text,
                ctx.edit_ctx,
                source_context.as_ref(),
            )
        } else {
            base_projection.clone()
        };
        if let Some(source_text) = ctx.source_text {
            append_trailing_whitespace_projection(&mut projected, source_text);
        }
        let marker = (line_idx == 0).then_some(ctx.active_block_marker.as_ref()).flatten();
        let marker_source_range =
            marker.map(|marker| marker_source_range_for_projected_line(&projected, marker));
        if let Some(marker) = marker {
            projected = materialize_marker_for_projected_line(
                projected,
                marker,
                marker_source_range
                    .clone()
                    .expect("an active marker must have a derived source range"),
            );
        }
        let materialized_spans = if let Some(source_text) = ctx.source_text {
            crate::edit::materialized_spans_for_projected_line(
                &base_projection,
                line_styles,
                source_text,
                ctx.edit_ctx,
            )
        } else {
            crate::edit::materialize_line(raw, line_styles, "", None).spans
        };
        let mut materialized_styles: Vec<StyleSpan> = materialized_spans
            .iter()
            .map(|span| StyleSpan {
                start: span.start,
                len: span.len,
                style: span.style.clone(),
                source_range: span.source_range.clone(),
            })
            .collect();
        if let Some(marker) = marker {
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
                    source_range: marker_source_range
                        .clone()
                        .expect("an active marker must have a derived source range"),
                },
            );
        }
        let wrapped = ctx.wrap_text(&projected.text, font_size, font_weight);
        let visual_grapheme_bytes = crate::grapheme_map::grapheme_byte_boundaries(&projected.text);

        let shaped_input = ctx.last_wrap_shaped.first().and_then(|s| s.as_ref());
        let font_family_str = ctx.style.body_font_family.first().map(|s| s.as_str());
        for w in &wrapped {
            let seg_start = w.byte_start;
            let seg_end = w.byte_end;
            // Extract style spans that fall within this wrapped segment
            let mut seg_styles = Vec::new();
            for span in &materialized_styles {
                let span_end = span.start + span.len;
                if span_end <= seg_start || span.start >= seg_end {
                    continue;
                }
                let clamp_start = span.start.max(seg_start) - seg_start;
                let clamp_end = span_end.min(seg_end) - seg_start;
                seg_styles.push(StyleSpan {
                    start: clamp_start,
                    len: clamp_end - clamp_start,
                    style: span.style.clone(),
                    source_range: span.source_range.clone(),
                });
            }
            let seg_text_layout = if let Some(full) = shaped_input {
                super::shaping::segment_text_layout(
                    full,
                    seg_start,
                    seg_end,
                    &w.text,
                    font_size,
                    font_family_str,
                    font_weight,
                )
            } else {
                None
            };
            let source_projection = projected
                .slice_visual_line_indexed(&visual_grapheme_bytes, 0, seg_start..seg_end)
                .expect("wrapped visual lines must end at projection grapheme boundaries");
            laid_out_lines.push(LaidOutLine {
                text: w.text.clone(),
                rect: ui::core::geom::Rect::new(ctx.indent, ly, ctx.available_width(), line_h),
                font_size,
                is_code: false,
                font_weight,
                color_override: Some(if ctx.color_fade > 0.0 {
                    crate::style::blend_toward_bg(color, ctx.style.background_color, ctx.color_fade)
                } else {
                    color
                }),
                doc_line_idx: line_idx,
                styles: seg_styles.clone(),
                style_segments: vec![],
                shaped: None,
                text_layout: seg_text_layout,
                highlight_spans: vec![],
                source_projection: Some(source_projection),
            });
            ly += line_h;
        }
    }

    let total_h =
        if laid_out_lines.is_empty() { line_h } else { laid_out_lines.len() as f32 * line_h };
    ctx.push_block(LaidOutBlockKind::Text { lines: laid_out_lines }, total_h);
}

fn append_trailing_whitespace_projection(
    projected: &mut crate::projection::ProjectedText,
    source_text: &str,
) {
    if projected.text.trim().is_empty() {
        return;
    }
    let Some(last_boundary) = projected.boundaries.last() else {
        return;
    };
    let source_line_end = source_text[last_boundary.byte..]
        .find('\n')
        .map_or(source_text.len(), |newline_offset| last_boundary.byte + newline_offset);
    let trailing_range = last_boundary.byte..source_line_end;
    let Some(trailing_text) = source_text.get(trailing_range.clone()) else {
        return;
    };
    if trailing_text.is_empty() || !trailing_text.chars().all(char::is_whitespace) {
        return;
    }
    let visual_end = projected.text.len();
    projected.spans.push(crate::projection::ProjectionSpan {
        source_range: trailing_range,
        visual_range: visual_end..visual_end,
        kind: crate::projection::ProjectionSpanKind::Collapsed,
    });
}

pub(crate) fn layout_table(block: &BlockNode, ctx: &mut LayoutCtx, columns: usize) {
    let font_size = ctx.style.body_font_size;
    let line_h = ctx.style.line_height;
    let pad = ctx.style.table_cell_padding;
    let available_w = ctx.available_width().max(20.0);

    // Dynamic column width: measure content demand, then allocate proportionally
    let demand =
        measure_column_demand(block, columns, font_size, ctx.shaper.as_deref_mut(), ctx.doc);
    let min_col_w = font_size * 3.0; // at least 3 characters wide
    // For 2-column tables, allow a column to use most of the space (leaving
    // at least min_col_w for the other).  For 3+ columns, cap at 60 % so no
    // single column hogs the table.  The .max() fallback keeps the 60 % floor
    // for extremely narrow viewports where available_w < min_col_w / 0.4.
    let max_col_w = if columns == 2 {
        (available_w - min_col_w).max(available_w * 0.6)
    } else {
        available_w * 0.6
    };
    let column_widths = allocate_column_widths(&demand, available_w, pad, min_col_w, max_col_w);

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
                let projected = cell.projected_lines.get(t_idx).cloned().unwrap_or_else(|| {
                    crate::projection::ProjectedText::direct(t, cell.block_range.start)
                });
                let cell_x = col_x + pad;
                let cell_inner_w = (cell_w - pad * 2.0).max(1.0);
                let wrapped = ctx.wrap_text_with_width(t, font_size, Weight::NORMAL, cell_inner_w);
                let mut laid = layout_line_with_styles(
                    line_styles,
                    &wrapped,
                    &projected,
                    font_size,
                    line_h,
                    cell_x,
                    cy,
                    cell_inner_w,
                    ctx.style.text_color,
                    ctx.style.body_font_family.first().map(|s| s.as_str()),
                    ctx.shaper.as_deref_mut(),
                    t_idx,
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
                laid_out.extend(laid);
                cy += line_h * n as f32;
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
///   - the longest non-breakable token width (space-delimited)
///   - the longest full-line width × 0.6
/// This ensures narrow content columns don't hog space while wide columns
/// get enough room to avoid excessive wrapping.
pub(crate) fn measure_column_demand(
    block: &BlockNode,
    columns: usize,
    font_size: f32,
    mut shaper: Option<&mut Shaper>,
    doc: &dyn core::document::DocView,
) -> Vec<f32> {
    let mut demand = vec![0.0f32; columns];

    // Helper: update demand for a single cell's text content.
    let measure_cell = |texts: &[std::borrow::Cow<'_, str>],
                        ci: usize,
                        demand: &mut Vec<f32>,
                        shaper: &mut Option<&mut Shaper>,
                        font_size: f32| {
        for t in texts {
            if t.is_empty() {
                continue;
            }
            let (max_token_w, full_w) = shaper
                .as_mut()
                .map(|s| {
                    s.set_font_size(font_size);
                    let max_tok = t
                        .split(' ')
                        .filter_map(|tok| s.shape(tok).ok().map(|r| r.width))
                        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                        .unwrap_or(0.0);
                    let full = s.shape(t).ok().map(|r| r.width).unwrap_or(0.0);
                    (max_tok, full)
                })
                .unwrap_or_else(|| {
                    let char_est = |tok: &str| tok.chars().count() as f32 * font_size * 0.55;
                    let max_tok = t
                        .split(' ')
                        .map(&char_est)
                        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                        .unwrap_or(0.0);
                    (max_tok, char_est(t))
                });
            let d = max_token_w.max(full_w * 0.6);
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
            let (texts, _) = collect_text_lines_with_styles(child, doc);
            measure_cell(&texts, col, &mut demand, &mut shaper, font_size);
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
            let (texts, _) = collect_text_lines_with_styles(cell, doc);
            measure_cell(&texts, ci, &mut demand, &mut shaper, font_size);
        }
    }
    demand
}

/// Allocate column widths from content demand and available space.
///
/// Each column gets at least `min_col_w` and at most `max_col_w` (before padding).
/// Remaining space is distributed proportionally to demand. If clamping creates
/// surplus or deficit, a second pass redistributes among eligible columns.
pub(crate) fn allocate_column_widths(
    demand: &[f32],
    available_w: f32,
    pad: f32,
    min_col_w: f32,
    max_col_w: f32,
) -> Vec<f32> {
    let cols = demand.len();
    if cols == 0 {
        return vec![];
    }

    // Single column: use full available width
    if cols == 1 {
        return vec![available_w];
    }

    let total_pad = pad * 2.0 * cols as f32;
    let net_w = (available_w - total_pad).max(0.0);
    let total_demand: f32 = demand.iter().sum();

    // Empty table — equal distribution
    if total_demand <= 0.0 {
        return vec![net_w / cols as f32 + pad * 2.0; cols];
    }

    // First pass: proportional allocation with min/max clamping
    let mut widths: Vec<f32> = demand
        .iter()
        .map(|&d| {
            let w = (net_w * d / total_demand).max(min_col_w).min(max_col_w);
            w + pad * 2.0 // add cell padding back to get full column width
        })
        .collect();

    // Second pass: redistribute surplus/deficit from clamping
    let allocated: f32 = widths.iter().sum::<f32>() - total_pad;
    let delta = net_w - allocated;

    if delta.abs() > 0.5 {
        let eligible: Vec<usize> = (0..cols)
            .filter(|&i| {
                let w = widths[i] - pad * 2.0;
                if delta > 0.0 { w < max_col_w } else { w > min_col_w }
            })
            .collect();
        let eligible_demand: f32 = eligible.iter().map(|&i| demand[i]).sum();
        if eligible_demand > 0.0 {
            for &i in &eligible {
                let share = delta * demand[i] / eligible_demand;
                let new_w =
                    (widths[i] + share).max(min_col_w + pad * 2.0).min(max_col_w + pad * 2.0);
                widths[i] = new_w;
            }
        }
    }

    // Final normalization: if second pass left slack due to all columns
    // hitting constraints, distribute remaining space among expandable columns.
    let final_allocated: f32 = widths.iter().sum::<f32>() - total_pad;
    let slack = net_w - final_allocated;
    if slack.abs() > 0.5 {
        let expandable: Vec<usize> =
            (0..cols).filter(|&i| widths[i] - pad * 2.0 < max_col_w).collect();
        if !expandable.is_empty() {
            let per_col = slack / expandable.len() as f32;
            for &i in &expandable {
                widths[i] = (widths[i] + per_col).min(max_col_w + pad * 2.0);
            }
        }
    }

    widths
}

/// Layout a line of text with style spans, adjusting spans for wrapped segments.
/// Returns LaidOutLine list with correct style information.
///
/// Every wrapped segment receives a slice of the canonical source projection.
fn layout_line_with_styles(
    line_styles: &[StyleSpan],
    wrapped: &[super::types::WrappedLine],
    projected: &crate::projection::ProjectedText,
    font_size: f32,
    line_h: f32,
    x: f32,
    y_start: f32,
    width: f32,
    color: [f32; 4],
    _font_family: Option<&str>,
    mut _shaper: Option<&mut Shaper>,
    doc_line_idx: usize,
) -> Vec<LaidOutLine> {
    let mut result = Vec::new();
    let mut ly = y_start;
    let visual_grapheme_bytes = crate::grapheme_map::grapheme_byte_boundaries(&projected.text);
    for w in wrapped {
        let seg_start = w.byte_start;
        let seg_end = w.byte_end;
        let mut seg_styles = Vec::new();
        for span in line_styles {
            let span_end = span.start + span.len;
            if span_end <= seg_start || span.start >= seg_end {
                continue;
            }
            let clamp_start = span.start.max(seg_start) - seg_start;
            let clamp_end = span_end.min(seg_end) - seg_start;
            seg_styles.push(StyleSpan {
                start: clamp_start,
                len: clamp_end - clamp_start,
                style: span.style.clone(),
                source_range: span.source_range.clone(),
            });
        }
        let source_projection = projected
            .slice_visual_line_indexed(&visual_grapheme_bytes, 0, seg_start..seg_end)
            .expect("wrapped visual lines must end at projection grapheme boundaries");
        // Shaping deferred to render phase (only visible lines)
        result.push(LaidOutLine {
            text: w.text.clone(),
            rect: ui::core::geom::Rect::new(x, ly, width, line_h),
            font_size,
            is_code: false,
            font_weight: Weight::NORMAL,
            color_override: Some(color),
            doc_line_idx,
            styles: seg_styles,
            style_segments: vec![],
            shaped: None,
            text_layout: None,
            highlight_spans: vec![],
            source_projection: Some(source_projection),
        });
        ly += line_h;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::MarkdownDoc;
    use crate::layout::LazyLayout;
    use crate::parser::parse_markdown;
    use crate::test_utils::default_style;

    const ASCII_DIAGRAM_SOURCE: &str = "```\n┌────┐\n│中文│\n└────┘\n```";
    const INDENTED_ASCII_DIAGRAM_SOURCE: &str = "    ┌────┐\n    │中文│\n    └────┘";

    fn make_doc(md: &str) -> (&str, MarkdownDoc) {
        let parsed = parse_markdown(md);
        (md, MarkdownDoc::build(&parsed, &default_style()))
    }

    fn find_line_in_block<'a>(block: &'a LaidOutBlock, needle: &str) -> Option<&'a LaidOutLine> {
        match &block.kind {
            LaidOutBlockKind::Text { lines }
            | LaidOutBlockKind::MetadataBlock { lines }
            | LaidOutBlockKind::CodeBlock { lines, .. } => {
                lines.iter().find(|line| line.text.contains(needle))
            }
            LaidOutBlockKind::BlockQuote { blocks } => {
                blocks.iter().find_map(|child| find_line_in_block(child, needle))
            }
            LaidOutBlockKind::ListItem { lines, blocks, .. } => lines
                .iter()
                .find(|line| line.text.contains(needle))
                .or_else(|| blocks.iter().find_map(|child| find_line_in_block(child, needle))),
            LaidOutBlockKind::Table { header, rows, .. } => header
                .iter()
                .flatten()
                .chain(rows.iter().flatten().flatten())
                .find(|line| line.text.contains(needle)),
            LaidOutBlockKind::HorizontalRule => None,
        }
    }

    fn layout_with_cursor_and_width(
        source: &str,
        cursor_byte: usize,
        width: f32,
    ) -> LazyLayout<crate::builder::MarkdownDoc> {
        let parsed = crate::parser::parse_markdown(source);
        let style = default_style();
        let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
        let doc_view = core::document::StringDocView::new(source);
        let mut lazy = LazyLayout::from_doc(doc, &style, width, &doc_view);
        lazy.set_edit_source(Some(source.to_string()));
        lazy.set_edit_ctx(Some(crate::edit::EditContext {
            cursor_byte,
            preedit_text: None,
            preedit_cursor: None,
        }));
        let mut shaper = Shaper::new().expect("list projection test needs a shaper");
        lazy.ensure_precise_range(0.0, 600.0, &style, &mut shaper, None, &doc_view);
        lazy.build_flat_lines(&doc_view);
        lazy
    }

    fn layout_with_selection_and_width(
        source: &str,
        cursor_byte: usize,
        selection_range: std::ops::Range<usize>,
        width: f32,
    ) -> LazyLayout<crate::builder::MarkdownDoc> {
        let parsed = crate::parser::parse_markdown(source);
        let style = default_style();
        let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
        let doc_view = core::document::StringDocView::new(source);
        let mut lazy = LazyLayout::from_doc(doc, &style, width, &doc_view);
        lazy.set_edit_source(Some(source.to_string()));
        lazy.set_edit_ctx(Some(crate::edit::EditContext {
            cursor_byte,
            preedit_text: None,
            preedit_cursor: None,
        }));
        lazy.set_selection_range(Some(selection_range));
        let mut shaper = Shaper::new().expect("selection layout test needs a shaper");
        lazy.ensure_precise_range(0.0, 600.0, &style, &mut shaper, None, &doc_view);
        lazy.build_flat_lines(&doc_view);
        lazy
    }

    fn layout_doc_with_width(source: &str, width: f32) -> LaidOutDoc {
        let parsed = crate::parser::parse_markdown(source);
        let style = default_style();
        let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
        let doc_view = core::document::StringDocView::new(source);
        let mut shaper = Shaper::new().expect("table projection test needs a shaper");
        layout_doc_with_shaper(&doc.blocks, &style, width, Some(&mut shaper), None, &doc_view)
    }

    fn layout_doc_with_registry_and_width(source: &str, width: f32) -> MarkdownLayout {
        let parsed = crate::parser::parse_markdown(source);
        let style = default_style();
        let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
        let doc_view = core::document::StringDocView::new(source);
        let mut shaper = Shaper::new().expect("ASCII diagram test needs a shaper");
        layout_doc_with_shaper_for_rendering(
            &doc.blocks,
            &style,
            width,
            Some(&mut shaper),
            None,
            &doc_view,
        )
    }

    #[test]
    fn layout_marks_non_active_box_diagram_code_block() {
        let laid_out = layout_doc_with_registry_and_width(ASCII_DIAGRAM_SOURCE, 400.0);
        let block = laid_out.doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
            panic!("fixture must produce a code block");
        };
        assert_eq!(
            laid_out.ascii_diagrams.diagram_for(lines).map(|diagram| diagram.column_count),
            Some(6)
        );
    }

    #[test]
    fn layout_does_not_mark_indented_box_diagram_code_block() {
        let laid_out = layout_doc_with_registry_and_width(INDENTED_ASCII_DIAGRAM_SOURCE, 400.0);
        let block = laid_out.doc.blocks.first().expect("fixture has one code block");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
            panic!("fixture must produce a code block");
        };
        assert!(
            laid_out.ascii_diagrams.diagram_for(lines).is_none(),
            "only fenced code blocks may enable the grid path"
        );
    }

    #[test]
    fn active_box_diagram_code_block_keeps_normal_layout_path() {
        let cursor_byte = ASCII_DIAGRAM_SOURCE.find("中文").expect("fixture has CJK label");
        let layout = layout_with_cursor_and_width(ASCII_DIAGRAM_SOURCE, cursor_byte, 400.0);
        let block = layout.laid_out[0].as_ref().expect("visible code block must materialize");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
            panic!("fixture must produce a code block");
        };
        assert!(
            layout.ascii_diagrams().diagram_for(lines).is_none(),
            "active code blocks must keep the existing path"
        );
    }

    #[test]
    fn selection_first_materialization_keeps_fence_lines_out_of_diagram_and_restores_grid() {
        let source = format!("{ASCII_DIAGRAM_SOURCE}\noutside");
        let selection_start = source.find("中文").expect("fixture has diagram text");
        let parsed = crate::parser::parse_markdown(&source);
        let style = default_style();
        let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
        let doc_view = core::document::StringDocView::new(&source);
        let mut layout = LazyLayout::new(doc, &style, 400.0, &doc_view);
        layout.set_edit_source(Some(source.clone()));
        layout.set_edit_ctx(Some(crate::edit::EditContext {
            cursor_byte: source.len(),
            preedit_text: None,
            preedit_cursor: None,
        }));
        layout.set_selection_range(Some(selection_start..source.len()));
        let mut shaper = Shaper::new().expect("selection materialization test needs a shaper");
        layout.ensure_visible(0.0, 600.0, &style, 400.0, &mut shaper, None, &doc_view);

        {
            let block = layout.laid_out[0].as_ref().expect("visible code block must materialize");
            let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
                panic!("fixture must produce a code block");
            };

            assert_ne!(lines.first().map(|line| line.text.as_str()), Some("```"));
            assert!(
                layout.ascii_diagrams().diagram_for(lines).is_none(),
                "selection must disable only grid rendering"
            );
        }

        layout.set_selection_range(None);

        let block = layout.laid_out[0].as_ref().expect("visible code block must materialize");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
            panic!("fixture must produce a code block");
        };

        assert_eq!(
            layout.ascii_diagrams().diagram_for(lines).map(|diagram| diagram.rows.len()),
            Some(3),
            "clearing selection must restore the original three-content-line diagram"
        );
    }

    #[test]
    fn selection_starting_inside_diagram_keeps_code_block_on_text_path() {
        let source = format!("{ASCII_DIAGRAM_SOURCE}\noutside");
        let selection_end = source.find("中文").expect("fixture has diagram text");
        let layout = layout_with_selection_and_width(
            &source,
            selection_end,
            selection_end..source.len(),
            400.0,
        );
        let block = layout.laid_out[0].as_ref().expect("visible code block must materialize");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
            panic!("fixture must produce a code block");
        };

        assert_eq!(lines.first().map(|line| line.text.as_str()), Some("```"));
    }

    #[test]
    fn zero_width_selection_keeps_non_active_diagram_grid_path() {
        let source = format!("{ASCII_DIAGRAM_SOURCE}\noutside");
        let selection_start = source.find("中文").expect("fixture has diagram text");
        let layout = layout_with_selection_and_width(
            &source,
            source.len(),
            selection_start..selection_start,
            400.0,
        );
        let block = layout.laid_out[0].as_ref().expect("visible code block must materialize");
        let LaidOutBlockKind::CodeBlock { lines, .. } = &block.kind else {
            panic!("fixture must produce a code block");
        };

        assert_ne!(lines.first().map(|line| line.text.as_str()), Some("```"));
        assert!(
            layout.ascii_diagrams().diagram_for(lines).is_some(),
            "zero-width selection must preserve the grid path"
        );
    }

    fn collect_laid_out_text(block: &LaidOutBlock, out: &mut String) {
        match &block.kind {
            LaidOutBlockKind::Text { lines } => {
                for line in lines {
                    out.push_str(&line.text);
                }
            }
            LaidOutBlockKind::CodeBlock { lines, .. } => {
                for line in lines {
                    out.push_str(&line.text);
                }
            }
            LaidOutBlockKind::Table { header, rows, .. } => {
                for cell in header {
                    for line in cell {
                        out.push_str(&line.text);
                    }
                }
                for row in rows {
                    for cell in row {
                        for line in cell {
                            out.push_str(&line.text);
                        }
                    }
                }
            }
            LaidOutBlockKind::ListItem { lines, blocks, .. } => {
                for line in lines {
                    out.push_str(&line.text);
                }
                for b in blocks {
                    collect_laid_out_text(b, out);
                }
            }
            LaidOutBlockKind::BlockQuote { blocks } => {
                for b in blocks {
                    collect_laid_out_text(b, out);
                }
            }
            LaidOutBlockKind::HorizontalRule => {}
            LaidOutBlockKind::MetadataBlock { lines } => {
                for l in lines {
                    out.push_str(&l.text);
                    out.push('\n');
                }
            }
        }
    }

    #[test]
    fn layout_paragraph_has_rect() {
        let (src, doc) = make_doc("hello world");
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        assert!(!laid_out.blocks.is_empty());
        assert!(laid_out.blocks[0].rect.w > 0.0);
        assert!(laid_out.blocks[0].rect.h > 0.0);
    }

    #[test]
    fn plain_shaped_text_keeps_projection_without_legacy_source_map() {
        let source = "plain paragraph";
        let (_, doc) = make_doc(source);
        let mut shaper = Shaper::new().expect("plain layout regression test needs a shaper");
        let laid_out = layout_doc_with_shaper(
            &doc.blocks,
            &default_style(),
            400.0,
            Some(&mut shaper),
            None,
            &core::document::StringDocView::new(source),
        );
        let lines = match &laid_out.blocks[0].kind {
            LaidOutBlockKind::Text { lines } => lines,
            _ => panic!("plain paragraph must lay out as text"),
        };

        assert!(
            lines.iter().all(|line| line.source_projection.is_some()),
            "plain shaped text must retain its canonical source projection"
        );
    }

    #[test]
    fn wrapped_table_cells_keep_distinct_source_extents() {
        let source = "| left header | right header |\n| --- | --- |\n| left body wraps here | right body wraps here |";
        let laid = layout_doc_with_width(source, 220.0);
        let left = laid
            .blocks
            .iter()
            .find_map(|block| find_line_in_block(block, "left body"))
            .expect("left body must be laid out");
        let right = laid
            .blocks
            .iter()
            .find_map(|block| find_line_in_block(block, "right body"))
            .expect("right body must be laid out");
        let left_projection = left.source_projection.as_ref().expect("left projection");
        let right_projection = right.source_projection.as_ref().expect("right projection");

        assert_ne!(left_projection.owner, right_projection.owner);
        assert!(left_projection.source_extent.end <= right_projection.source_extent.start);
    }

    #[test]
    fn multiline_list_item_projection_collapses_continuation_indent() {
        let source = "- first line\n  continuation line";
        let continuation = source.find("continuation").expect("fixture contains continuation");
        let lazy = layout_with_cursor_and_width(source, continuation, 400.0);
        let list_lines = lazy
            .laid_out
            .iter()
            .flatten()
            .find_map(|block| match &block.kind {
                LaidOutBlockKind::ListItem { lines, .. } => Some(lines),
                _ => None,
            })
            .expect("fixture must have a laid-out list item");
        assert_eq!(
            list_lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(),
            ["- first line continuation line"],
            "a list softbreak must remain in one logical line"
        );
        let marker_projection =
            list_lines[0].source_projection.as_ref().expect("marker projection");
        assert_eq!(
            marker_projection.boundaries[0].byte, 0,
            "the active list marker must anchor to its source byte"
        );
        let continuation_boundary = marker_projection
            .collapsed
            .first()
            .expect("the logical line must retain the collapsed softbreak");
        assert!(
            continuation_boundary.source_range.start < continuation
                && continuation_boundary.source_range.end <= continuation,
            "collapsed source range must cover the newline and continuation indent"
        );
        let line = lazy
            .laid_out
            .iter()
            .flatten()
            .find_map(|block| find_line_in_block(block, "continuation"))
            .expect("continuation must have a laid-out line");

        assert!(
            line.source_projection
                .as_ref()
                .expect("projection")
                .boundaries
                .iter()
                .any(|anchor| anchor.byte == continuation)
        );
    }

    #[test]
    fn nested_list_cursor_projection_belongs_to_inner_item() {
        let source = "- outer\n  - inner wrapped content wrapped content";
        let inner = source.find("inner").expect("fixture contains inner");
        let lazy = layout_with_cursor_and_width(source, inner, 140.0);

        assert!(
            lazy.flat_lines
                .iter()
                .filter(|line| line.text.contains("inner"))
                .all(|line| line.source_projection.is_some())
        );
    }

    #[test]
    fn nested_list_continuation_stays_logical_and_retains_collapsed_indent() {
        let source = "- outer\n  - **first\n    second**";
        let first = source.find("first").expect("fixture contains first line text");
        let second = source.find("second").expect("fixture contains continuation text");
        let continuation_newline =
            source[..second].rfind('\n').expect("fixture contains the continuation newline");
        let lazy = layout_with_cursor_and_width(source, second, 400.0);
        let nested_lines = lazy
            .flat_lines
            .iter()
            .filter(|line| line.text.contains("first") || line.text.contains("second"))
            .collect::<Vec<_>>();

        assert_eq!(
            nested_lines.len(),
            1,
            "nested-list softbreaks must stay in one logical line, got {:?}",
            nested_lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>()
        );
        assert!(
            nested_lines
                .iter()
                .all(|line| !line.text.is_empty() && line.source_projection.is_some()),
            "the nested-list logical line must keep its source projection"
        );

        let first_line = nested_lines
            .iter()
            .find(|line| line.text.contains("first"))
            .expect("the nested list line must be present");
        let collapsed = first_line
            .source_projection
            .as_ref()
            .expect("the nested list line needs a projection")
            .collapsed
            .first()
            .expect("the nested list line must retain its collapsed continuation");

        assert!(
            collapsed.source_range.start <= continuation_newline
                && collapsed.source_range.end >= second,
            "the collapsed range must cover the physical newline and continuation indent"
        );
        assert!(
            source[collapsed.source_range.clone()].starts_with("\n    "),
            "the collapsed range must retain the newline plus nested-list continuation indent"
        );
        assert!(first < second, "the fixture must keep the first line before its continuation");
    }

    #[test]
    fn styled_list_continuation_retains_collapsed_source_boundary() {
        let source = "- **first\n  second**";
        let second = source.find("second").expect("fixture contains continuation text");
        let lazy = layout_with_cursor_and_width(source, second, 400.0);
        let list_lines = lazy
            .laid_out
            .iter()
            .flatten()
            .find_map(|block| match &block.kind {
                LaidOutBlockKind::ListItem { lines, .. } => Some(lines),
                _ => None,
            })
            .expect("fixture must have a laid-out list item");
        let continuation = list_lines
            .iter()
            .find(|line| line.text.contains("second"))
            .expect("continuation must have a laid-out line");

        assert_eq!(
            list_lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(),
            ["- **first second**"],
            "active inline markers must preserve the collapsed logical line"
        );
        assert!(
            continuation
                .source_projection
                .as_ref()
                .expect("continuation projection")
                .boundaries
                .iter()
                .any(|anchor| anchor.byte == second),
            "continuation projection must retain the continuation source anchor"
        );
        let continuation_projection =
            continuation.source_projection.as_ref().expect("continuation projection");
        assert!(
            continuation_projection.source_extent.start <= second,
            "continuation projection must cover its collapsed source boundary"
        );
        assert!(
            continuation_projection
                .collapsed
                .iter()
                .any(|collapsed| collapsed.source_range.start < second
                    && collapsed.source_range.end <= second),
            "continuation projection must retain the newline and indentation mapping"
        );
        let bold = continuation
            .styles
            .iter()
            .find(|span| matches!(span.style, crate::builder::InlineStyle::Bold))
            .expect("continuation text must retain its bold style");
        assert_eq!(
            &continuation.text[bold.start..bold.start + bold.len],
            "first second",
            "the complete projected text must remain bold"
        );
        assert_eq!(
            continuation
                .styles
                .iter()
                .filter(|span| matches!(span.style, crate::builder::InlineStyle::SourceMarker))
                .map(|span| &continuation.text[span.start..span.start + span.len])
                .collect::<Vec<_>>(),
            ["- ", "**", "**"],
            "materialization must classify the list and inline delimiters as source markers"
        );
    }

    #[test]
    fn metadata_projection_starts_at_content_event_range() {
        let source = "---\ntitle: hello\n---";
        let (_, doc) = make_doc(source);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(source),
        );
        let metadata_lines = laid_out
            .blocks
            .iter()
            .find_map(|block| match &block.kind {
                LaidOutBlockKind::MetadataBlock { lines } => Some(lines),
                _ => None,
            })
            .expect("frontmatter fixture must produce a metadata block");
        let projection = metadata_lines[0]
            .source_projection
            .as_ref()
            .expect("metadata lines require a source projection");

        assert_eq!(
            projection.boundaries[0].byte,
            source.find("title: hello").expect("fixture must contain metadata content"),
        );
    }

    #[test]
    fn metadata_projection_uses_each_physical_line_event_range() {
        let source = "---\ntitle: hello\nauthor: textora\n---";
        let (_, doc) = make_doc(source);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(source),
        );
        let metadata_lines = laid_out
            .blocks
            .iter()
            .find_map(|block| match &block.kind {
                LaidOutBlockKind::MetadataBlock { lines } => Some(lines),
                _ => None,
            })
            .expect("frontmatter fixture must produce a metadata block");

        for line in metadata_lines {
            let projection = line
                .source_projection
                .as_ref()
                .expect("metadata lines require a source projection");
            assert_eq!(
                projection.boundaries[0].byte,
                source.find(&line.text).expect("metadata line must occur in source"),
            );
        }
    }

    #[test]
    fn layout_heading_larger_than_paragraph() {
        let (h1_src, h1) = make_doc("# Big Title");
        let (p_src, p) = make_doc("small text");
        let h1_layout = layout_doc(
            &h1.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(h1_src),
        );
        let p_layout = layout_doc(
            &p.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(p_src),
        );
        assert!(h1_layout.blocks[0].rect.h > p_layout.blocks[0].rect.h);
    }

    #[test]
    fn h1_ascii_text_wraps_at_correct_width() {
        let long_h1 = "# This is a very long heading that should wrap into multiple lines in a narrow viewport";
        let style = default_style();
        let h1_font_size = style.heading_font_sizes[0];
        let body_font_size = style.body_font_size;
        assert!(h1_font_size > body_font_size * 1.5, "H1 must be significantly larger than body");

        let (src, doc) = make_doc(long_h1);
        let laid_out =
            layout_doc(&doc.blocks, &style, 200.0, &core::document::StringDocView::new(src));
        let block = &laid_out.blocks[0];
        if let LaidOutBlockKind::Text { lines } = &block.kind {
            assert!(
                lines.len() >= 3,
                "H1 long ASCII text should wrap to 3+ lines at 200px viewport, got {} lines.                  Likely ascii_widths not scaled for heading font size.",
                lines.len()
            );
        } else {
            panic!("expected Text block for heading");
        }
    }

    #[test]
    fn layout_vertical_positions_increase() {
        let (src, doc) = make_doc("# A\n\n## B\n\nhello");
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        assert!(laid_out.blocks.len() >= 3);
        for i in 1..laid_out.blocks.len() {
            assert!(
                laid_out.blocks[i].rect.y >= laid_out.blocks[i - 1].rect.y,
                "block {} should be below block {}",
                i,
                i - 1
            );
        }
    }

    #[test]
    fn layout_total_height() {
        let (src, doc) = make_doc("# Title\n\nparagraph text here\n\n## Another heading");
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        assert!(laid_out.total_height > 0.0);
    }

    #[test]
    fn style_segments_empty_for_plain_text() {
        let (src, doc) = make_doc("hello world");
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let block = &laid_out.blocks[0];
        if let LaidOutBlockKind::Text { lines } = &block.kind {
            // Plain paragraph has no styled spans
            assert!(lines[0].style_segments.is_empty(), "plain text should have no style segments");
        }
    }

    #[test]
    fn style_segments_computed_for_inline_code() {
        let (src, doc) = make_doc("use `code` here");
        let style = default_style();
        let laid_out = layout_doc_with_shaper(
            &doc.blocks,
            &style,
            400.0,
            None,
            None,
            &core::document::StringDocView::new(src),
        );
        let block = &laid_out.blocks[0];
        if let LaidOutBlockKind::Text { lines } = &block.kind {
            // Without shaper, style_segments should be empty (fallback)
            assert!(lines[0].style_segments.is_empty(), "no shaper -> no segments");
        }
    }

    #[test]
    fn layout_cjk_text_no_panic() {
        let (src, doc) = make_doc(
            "需保留 SidebarPersistent 机制（inject_persistent 在生产中使用，用于跨帧保持隐藏/显示状态）",
        );
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            200.0,
            &core::document::StringDocView::new(src),
        );
        assert!(!laid_out.blocks.is_empty());
        assert!(laid_out.total_height > 0.0);
    }

    #[test]
    fn layout_long_cjk_line_wraps() {
        let long = "这是一段很长的中文文本用来测试自动换行功能是否正常工作不会产生恐慌";
        let (src, doc) = make_doc(long);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            150.0,
            &core::document::StringDocView::new(src),
        );
        // Should wrap into multiple lines
        let text_block =
            laid_out.blocks.iter().find(|b| matches!(&b.kind, LaidOutBlockKind::Text { .. }));
        assert!(text_block.is_some());
    }

    #[test]
    fn heading_spacing_collapses_between_adjacent_headings() {
        let (src, doc) = make_doc("## First\n### Second");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
        assert!(laid_out.blocks.len() >= 2, "should have 2 heading blocks");
        let h1_bottom = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h;
        let h2_top = laid_out.blocks[1].rect.y;
        let gap = h2_top - h1_bottom;
        // Adjacent headings: gap should be heading_spacing_bottom, not
        // heading_spacing_bottom + heading_spacing_top
        let expected_max = style.heading_spacing_bottom + 1.0; // small tolerance
        assert!(
            gap <= expected_max,
            "adjacent heading gap {} should collapse to ~{}, not {}",
            gap,
            style.heading_spacing_bottom,
            style.heading_spacing_bottom + style.heading_spacing_top
        );
    }

    #[test]
    fn first_heading_top_spacing_halved() {
        let (src, doc) = make_doc("# Title");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
        assert!(!laid_out.blocks.is_empty());
        let block = &laid_out.blocks[0];
        // First heading's y should be heading_spacing_top * 0.5
        let expected_y = style.heading_spacing_top * 0.5;
        assert!(
            (block.rect.y - expected_y).abs() < 1.0,
            "first heading y={} should be ~{}",
            block.rect.y,
            expected_y
        );
    }

    #[test]
    fn non_first_heading_has_full_top_spacing() {
        let (src, doc) = make_doc("paragraph\n\n# Heading");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
        assert!(laid_out.blocks.len() >= 2);
        let heading = &laid_out.blocks[1];
        let para_bottom = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h;
        let gap = heading.rect.y - para_bottom;
        // Should include heading_spacing_top (not collapsed, not halved)
        assert!(
            gap >= style.heading_spacing_top - 1.0,
            "non-first heading gap {} should include full top spacing {}",
            gap,
            style.heading_spacing_top
        );
    }

    #[test]
    fn blockquote_text_color_is_faded() {
        let (src, doc) = make_doc("> quoted text");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
        assert!(!laid_out.blocks.is_empty());
        // The blockquote block should contain child text blocks with faded color
        if let LaidOutBlockKind::BlockQuote { blocks } = &laid_out.blocks[0].kind {
            assert!(!blocks.is_empty(), "blockquote should have child blocks");
            if let LaidOutBlockKind::Text { lines } = &blocks[0].kind {
                let faded = lines[0].color_override.unwrap();
                let base = style.text_color;
                // Faded color should differ from base text color
                assert_ne!(faded, base, "blockquote text should be faded");
                // Alpha must remain 1.0
                assert_eq!(faded[3], 1.0, "alpha must be 1.0 for subpixel rendering");
                // RGB should be shifted toward background
                let bg = style.background_color;
                // Faded R should be between base R and bg R
                if base[0] < bg[0] {
                    assert!(faded[0] > base[0] && faded[0] <= bg[0]);
                } else {
                    assert!(faded[0] < base[0] && faded[0] >= bg[0]);
                }
            } else {
                panic!("blockquote child should be Text");
            }
        } else {
            panic!("first block should be BlockQuote");
        }
    }

    #[test]
    fn blockquote_nested_in_list_preserves_color_fade() {
        let (src, doc) = make_doc("- > quoted in list");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
        // Find the blockquote inside the list item
        fn find_blockquote(blocks: &[LaidOutBlock]) -> Option<&LaidOutBlock> {
            for b in blocks {
                if matches!(&b.kind, LaidOutBlockKind::BlockQuote { .. }) {
                    return Some(b);
                }
                if let LaidOutBlockKind::ListItem { blocks: sub, .. } = &b.kind
                    && let Some(found) = find_blockquote(sub)
                {
                    return Some(found);
                }
            }
            None
        }
        let bq = find_blockquote(&laid_out.blocks).expect("should find blockquote in list");
        if let LaidOutBlockKind::BlockQuote { blocks } = &bq.kind
            && let LaidOutBlockKind::Text { lines } = &blocks[0].kind
        {
            let faded = lines[0].color_override.unwrap();
            let base = style.text_color;
            assert_ne!(faded, base, "blockquote-in-list text should be faded");
            assert_eq!(faded[3], 1.0);
        }
    }

    #[test]
    fn nested_list_has_increasing_depth() {
        let (src, doc) = make_doc("- top\n  - nested\n    - deep");
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        // Find all ListItem blocks and check their depth
        fn find_depths(blocks: &[LaidOutBlock], depths: &mut Vec<usize>) {
            for b in blocks {
                if let LaidOutBlockKind::ListItem { depth, blocks: sub, .. } = &b.kind {
                    depths.push(*depth);
                    find_depths(sub, depths);
                }
            }
        }
        let mut depths = Vec::new();
        find_depths(&laid_out.blocks, &mut depths);
        assert!(depths.len() >= 3, "should have at least 3 list items");
        assert_eq!(depths[0], 0, "top-level depth should be 0");
        assert_eq!(depths[1], 1, "nested depth should be 1");
        assert_eq!(depths[2], 2, "deeply nested depth should be 2");
    }

    #[test]
    fn layout_table_has_rows() {
        let md = "| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let table = laid_out.blocks.iter().find_map(|b| {
            if let LaidOutBlockKind::Table { rows, .. } = &b.kind { Some(rows.len()) } else { None }
        });
        assert_eq!(table, Some(2), "table should have 2 body rows");
    }

    #[test]
    fn layout_table_row_heights_match_row_count() {
        let md = "| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let table = laid_out.blocks.iter().find_map(|b| {
            if let LaidOutBlockKind::Table { rows, row_heights, .. } = &b.kind {
                Some((rows.len(), row_heights.len()))
            } else {
                None
            }
        });
        assert_eq!(table, Some((2, 2)), "row_heights must have one entry per row");
    }

    #[test]
    fn layout_table_header_height_nonzero() {
        let md = "| Name | Value |\n| --- | --- |\n| x | 1 |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let header_h = laid_out.blocks.iter().find_map(|b| {
            if let LaidOutBlockKind::Table { header_height, .. } = &b.kind {
                Some(*header_height)
            } else {
                None
            }
        });
        assert!(header_h.unwrap_or(0.0) > 0.0, "header should have nonzero height");
    }

    #[test]
    fn layout_table_row_height_reflects_long_content() {
        // Single-column table with a very long cell that should wrap
        let md = "| Long text |\n| --- |\n| this is a very long piece of text that should wrap to multiple lines in a narrow column |";
        let (src, doc) = make_doc(md);
        // Use a narrow viewport to force wrapping
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            200.0,
            &core::document::StringDocView::new(src),
        );
        let row_h = laid_out.blocks.iter().find_map(|b| {
            if let LaidOutBlockKind::Table { row_heights, .. } = &b.kind {
                row_heights.first().copied()
            } else {
                None
            }
        });
        // Single line_height is 24px; wrapped content should be taller
        assert!(
            row_h.unwrap_or(0.0) > 30.0,
            "wrapped cell row height ({}) should exceed single line",
            row_h.unwrap_or(0.0)
        );
    }

    #[test]
    fn layout_table_column_widths_sum_to_available() {
        let md = "| a | b | c |\n| --- | --- | --- |\n| 1 | 2 | 3 |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let total_w: f32 = laid_out
            .blocks
            .iter()
            .filter_map(|b| {
                if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                    Some(column_widths.iter().sum::<f32>())
                } else {
                    None
                }
            })
            .sum();
        // Available width is 400. Columns may not exactly equal 400 due to
        // min/max clamping and padding, but should be within 10px.
        assert!(
            (total_w - 400.0).abs() < 10.0,
            "column widths ({}) should approximately fill available width (400)",
            total_w
        );
    }

    #[test]
    fn layout_table_wide_column_gets_more_space() {
        // Column 0: short, Column 1: very long
        let md = "| id | description |\n| --- | --- |\n| 1 | this is a very long description that needs more horizontal room |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let widths: Vec<f32> = laid_out
            .blocks
            .iter()
            .filter_map(|b| {
                if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                    Some(column_widths.clone())
                } else {
                    None
                }
            })
            .flatten()
            .collect();
        assert!(widths.len() >= 2, "expected at least 2 columns, got {}", widths.len());
        if widths.len() >= 2 {
            assert!(
                widths[1] > widths[0],
                "wide-content column 1 ({}px) should be wider than narrow column 0 ({}px)",
                widths[1],
                widths[0]
            );
        }
    }

    #[test]
    fn layout_table_ascii_art_not_broken_arbitrarily() {
        let md = "| File tree |\n| --- |\n| ├── mod.rs        # Widget trait |\n| └── state.rs      # State management |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let max_demand: f32 = laid_out
            .blocks
            .iter()
            .filter_map(|b| {
                if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                    column_widths.first().copied()
                } else {
                    None
                }
            })
            .sum();
        assert!(
            max_demand > 300.0,
            "single wide column should get most of the 400px viewport, got {}",
            max_demand
        );
    }

    #[test]
    fn layout_table_header_only_no_body() {
        let md = "| A | B |
| --- | --- |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let table = laid_out.blocks.iter().find_map(|b| {
            if let LaidOutBlockKind::Table { rows, header_height, .. } = &b.kind {
                Some((rows.len(), *header_height))
            } else {
                None
            }
        });
        let (body_count, hdr_h) = table.unwrap_or((99, 0.0));
        assert_eq!(body_count, 0, "header-only table should have 0 body rows");
        assert!(hdr_h > 0.0, "header height should be nonzero");
    }

    #[test]
    fn layout_table_cjk_content() {
        let md = "| 名前 | 説明 |
| --- | --- |
| 太郎 | これは長い日本語の説明文です。テーブル内で正しく折り返されるべきです。 |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let table = laid_out.blocks.iter().find_map(|b| {
            if let LaidOutBlockKind::Table { rows, row_heights, column_widths, .. } = &b.kind {
                Some((rows.len(), row_heights.len(), column_widths.len()))
            } else {
                None
            }
        });
        let (body_rows, rh_count, col_count) = table.unwrap_or((0, 0, 0));
        assert!(body_rows > 0, "should have body rows");
        assert_eq!(body_rows, rh_count, "row_heights count must match body rows");
        assert!(col_count >= 2, "should have at least 2 columns");
    }

    #[test]
    fn layout_table_many_columns() {
        let md = "| A | B | C | D | E |
| --- | --- | --- | --- | --- |
| 1 | 2 | 3 | 4 | 5 |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let widths = laid_out
            .blocks
            .iter()
            .find_map(|b| {
                if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                    Some(column_widths.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        assert_eq!(widths.len(), 5, "should have 5 columns");
        let total: f32 = widths.iter().sum();
        assert!(
            (total - 400.0).abs() < 10.0,
            "5-column widths ({}) should approximately fill available width",
            total
        );
        for (i, w) in widths.iter().enumerate() {
            assert!(*w > 5.0, "column {} width {} is too small", i, w);
        }
    }

    #[test]
    fn layout_table_cell_inner_w_minimum() {
        let md = "| a | b |
| --- | --- |
| x | y |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            30.0,
            &core::document::StringDocView::new(src),
        );
        let has_table =
            laid_out.blocks.iter().any(|b| matches!(b.kind, LaidOutBlockKind::Table { .. }));
        assert!(has_table, "should produce a table even with narrow viewport");
    }

    #[test]
    fn layout_table_header_cell_demand_affects_width() {
        let md = "| Very Long Header Title |
| --- |
| hi |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let width = laid_out
            .blocks
            .iter()
            .find_map(|b| {
                if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                    column_widths.first().copied()
                } else {
                    None
                }
            })
            .unwrap_or(0.0);
        assert!(
            width > 350.0,
            "single column with long header should get most of 400px, got {}",
            width
        );
    }

    #[test]
    fn layout_table_two_columns_unequal_content() {
        let md = "| Name | Description \n| --- | --- \n| Alice | This is a fairly long description that should ideally get more horizontal space so it does not wrap excessively |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let widths = laid_out
            .blocks
            .iter()
            .find_map(|b| {
                if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                    Some(column_widths.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        assert!(widths.len() >= 2, "expected 2 columns, got {}", widths.len());
        let total: f32 = widths.iter().sum();
        let narrow_frac = widths[0] / total;
        assert!(
            narrow_frac < 0.30,
            "narrow column took {:.0}% of space ({}px / {}px); expected < 30%",
            narrow_frac * 100.0,
            widths[0],
            total
        );
        assert!(
            widths[1] > widths[0] * 2.5,
            "wide column ({}px) should be >2.5x narrow column ({}px)",
            widths[1],
            widths[0]
        );
    }

    #[test]
    fn layout_table_two_columns_equal_content() {
        let md = "| Key | Value
| --- | ---
| foo | bar
| baz | qux |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let widths = laid_out
            .blocks
            .iter()
            .find_map(|b| {
                if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                    Some(column_widths.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        assert!(widths.len() >= 2, "expected 2 columns");
        let total: f32 = widths.iter().sum();
        let ratio = widths[0] / total;
        assert!(
            ratio > 0.35 && ratio < 0.65,
            "equal-content columns should be ~50/50, got {:.0}% / {:.0}%",
            ratio * 100.0,
            (1.0 - ratio) * 100.0
        );
    }

    #[test]
    fn layout_table_two_columns_extreme_ratio() {
        let md = "| Tag | Notes
| --- | ---
| x | This is an extremely long piece of text that should dominate the available horizontal space in the table layout |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let widths = laid_out
            .blocks
            .iter()
            .find_map(|b| {
                if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                    Some(column_widths.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        assert!(widths.len() >= 2, "expected 2 columns");
        let total: f32 = widths.iter().sum();
        assert!(
            widths[0] / total < 0.20,
            "near-empty column took {:.0}%; expected < 20%",
            widths[0] / total * 100.0
        );
    }

    #[test]
    fn layout_table_three_columns_cap_still_60_percent() {
        let md = "| A | B | C
| --- | --- | ---
| short | short | This is a very long description that used to be capped at 60 percent and should still be capped |";
        let (src, doc) = make_doc(md);
        let laid_out = layout_doc(
            &doc.blocks,
            &default_style(),
            400.0,
            &core::document::StringDocView::new(src),
        );
        let widths = laid_out
            .blocks
            .iter()
            .find_map(|b| {
                if let LaidOutBlockKind::Table { column_widths, .. } = &b.kind {
                    Some(column_widths.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        assert_eq!(widths.len(), 3, "expected 3 columns");
        let total: f32 = widths.iter().sum();
        for (i, w) in widths.iter().enumerate() {
            assert!(
                w / total < 0.65,
                "column {} took {:.0}% — 60%% cap violated",
                i,
                w / total * 100.0
            );
        }
    }

    #[test]
    fn layout_inline_code_double_space_no_panic() {
        let md = "| 状图（├── `mod.rs  # comment`）整行 |\n|---|\n| x |";
        let style = default_style();
        let dl = crate::render_markdown(md, &style, 400.0, 600.0, 0.0);
        assert!(!dl.cmds.is_empty());
    }

    #[test]
    fn layout_text_preserves_all_chars_after_viewport_resize() {
        let md = "| 状图（├── `mod.rs  # comment`）整行 |\n|---|\n| x |";
        let style = default_style();
        for &w in &[100.0, 188.0, 300.0, 500.0, 800.0] {
            let doc =
                crate::builder::MarkdownDoc::build(&crate::parser::parse_markdown(md), &style);
            let laid = layout_doc(&doc.blocks, &style, w, &core::document::StringDocView::new(md));
            let mut all_text = String::new();
            for block in &laid.blocks {
                collect_laid_out_text(block, &mut all_text);
            }
            assert!(all_text.contains("状图（├──"), "missing CJK at w={}: {}", w, all_text);
            assert!(all_text.contains("mod.rs"), "missing mod.rs at w={}: {}", w, all_text);
            assert!(all_text.contains("# comment"), "missing comment at w={}: {}", w, all_text);
            assert!(all_text.contains("）整行"), "missing closing at w={}: {}", w, all_text);
        }
    }

    #[test]
    fn list_spacing_not_added_across_paragraph() {
        let md = "- first\n\nparagraph\n\n- second";
        let parsed = crate::parser::parse_markdown(md);
        let style = default_style();
        let doc = crate::builder::MarkdownDoc::build(&parsed, &style);
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(md));
        let second_item = laid_out.blocks.iter().find(|b| {
            if let LaidOutBlockKind::ListItem { lines, .. } = &b.kind {
                lines.iter().any(|l| l.text.contains("second"))
            } else {
                false
            }
        });
        assert!(second_item.is_some(), "should find 'second' list item");
        let para_block = laid_out.blocks.iter().find(|b| {
            if let LaidOutBlockKind::Text { lines } = &b.kind {
                lines.iter().any(|l| l.text.contains("paragraph"))
            } else {
                false
            }
        });
        assert!(para_block.is_some(), "should find paragraph block");
        let gap =
            second_item.unwrap().rect.y - (para_block.unwrap().rect.y + para_block.unwrap().rect.h);
        let expected_gap = style.paragraph_spacing;
        assert!(
            (gap - expected_gap).abs() < 1.0,
            "gap between paragraph and non-adjacent list should be ~paragraph_spacing={}, got {}",
            expected_gap,
            gap
        );
    }

    #[test]
    fn heading_followed_by_code_block_has_spacing() {
        let (src, doc) = make_doc(
            "### 1.2 关键不变量\n\n```text\nΣ entry.visual_line_count = display_map.tree.total_rows()\n```",
        );
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 600.0, &core::document::StringDocView::new(src));
        assert!(laid_out.blocks.len() >= 2, "should have heading and code block");

        let heading = &laid_out.blocks[0];
        let code_block = &laid_out.blocks[1];

        let heading_bottom = heading.rect.y + heading.rect.h;
        let code_top = code_block.rect.y;

        let gap = code_top - heading_bottom;
        assert!(
            gap >= style.heading_spacing_bottom - 1.0,
            "gap {} between heading bottom and code block top should be >= heading_spacing_bottom {}",
            gap,
            style.heading_spacing_bottom
        );
    }

    #[test]
    fn heading_followed_by_list_spacing_not_too_large() {
        let (src, doc) = make_doc("## 标题\n\n- 列表项1\n- 列表项2");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 600.0, &core::document::StringDocView::new(src));
        assert!(laid_out.blocks.len() >= 2, "should have heading + list items");

        let heading = &laid_out.blocks[0];
        let heading_bottom = heading.rect.y + heading.rect.h;

        let first_item =
            laid_out.blocks.iter().find(|b| matches!(&b.kind, LaidOutBlockKind::ListItem { .. }));
        let first_item = first_item.expect("should have a list item");
        let item_top = first_item.rect.y;

        let gap = item_top - heading_bottom;
        let max_expected = style.heading_spacing_bottom + 2.0;
        assert!(
            gap <= max_expected,
            "gap {} between heading and list should be ~heading_spacing_bottom ({}), not {}",
            gap,
            style.heading_spacing_bottom,
            gap
        );
        assert!(
            gap >= style.heading_spacing_bottom - 1.0,
            "gap {} should be at least heading_spacing_bottom {}",
            gap,
            style.heading_spacing_bottom
        );
    }

    #[test]
    fn blockquote_height_excludes_trailing_paragraph_spacing() {
        let (src, doc) = make_doc("> quoted text");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
        assert!(!laid_out.blocks.is_empty(), "should have blockquote block");
        let bq = &laid_out.blocks[0];
        if let LaidOutBlockKind::BlockQuote { blocks } = &bq.kind {
            assert!(!blocks.is_empty(), "blockquote should have child blocks");
            let child_h: f32 = blocks.iter().map(|b| b.rect.h).sum();
            let expected_h = style.blockquote_padding + child_h + style.blockquote_padding;
            assert!(
                (bq.rect.h - expected_h).abs() < 1.0,
                "blockquote height {} should be ~{} (2*padding + child_h), diff={}",
                bq.rect.h,
                expected_h,
                bq.rect.h - expected_h
            );
        } else {
            panic!("expected BlockQuote");
        }
    }

    #[test]
    fn blockquote_has_spacing_to_next_block() {
        let (src, doc) = make_doc("> quoted text\n\nNext paragraph.");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
        assert!(laid_out.blocks.len() >= 2, "need blockquote + paragraph");
        let bq = &laid_out.blocks[0];
        let next = &laid_out.blocks[1];
        let bq_bottom = bq.rect.y + bq.rect.h;
        let gap = next.rect.y - bq_bottom;
        assert!(
            gap >= style.paragraph_spacing - 1.0,
            "gap between blockquote bottom and next block {} should be >= paragraph_spacing {}",
            gap,
            style.paragraph_spacing
        );
    }

    // ===== Margin collapsing tests =====

    #[test]
    fn para_heading_margin_collapsing() {
        let (src, doc) = make_doc("paragraph\n\n# Heading");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
        assert!(laid_out.blocks.len() >= 2);
        let para_bottom = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h;
        let heading_top = laid_out.blocks[1].rect.y;
        let gap = heading_top - para_bottom;
        let expected = style.heading_spacing_top;
        assert!(
            (gap - expected).abs() < 1.0,
            "para→H1 gap {} should be ~heading_spacing_top ({}), not para+top ({})",
            gap,
            expected,
            style.paragraph_spacing + style.heading_spacing_top
        );
    }

    #[test]
    fn code_heading_margin_collapsing() {
        let (src, doc) = make_doc("```\ncode\n```\n\n## Heading");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
        assert!(laid_out.blocks.len() >= 2);
        let code_block = laid_out
            .blocks
            .iter()
            .find(|b| matches!(&b.kind, LaidOutBlockKind::CodeBlock { .. }))
            .unwrap();
        let code_bottom = code_block.rect.y + code_block.rect.h;
        let heading = laid_out
            .blocks
            .iter()
            .find(|b| matches!(&b.kind, LaidOutBlockKind::Text { .. }))
            .unwrap();
        let gap = heading.rect.y - code_bottom;
        let expected = style.heading_spacing_top * 0.8;
        assert!(
            (gap - expected).abs() < 1.0,
            "code→H2 gap {} should be ~{} (margin collapsing), not {}",
            gap,
            expected,
            style.paragraph_spacing + expected
        );
    }

    #[test]
    fn code_to_list_not_double_spaced() {
        let (src, doc) = make_doc("```\ncode\n```\n\n- item");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
        let code_block = laid_out
            .blocks
            .iter()
            .find(|b| matches!(&b.kind, LaidOutBlockKind::CodeBlock { .. }))
            .unwrap();
        let code_bottom = code_block.rect.y + code_block.rect.h;
        let list_item = laid_out
            .blocks
            .iter()
            .find(|b| matches!(&b.kind, LaidOutBlockKind::ListItem { .. }))
            .unwrap();
        let gap = list_item.rect.y - code_bottom;
        let expected = style.paragraph_spacing;
        assert!(
            (gap - expected).abs() < 1.0,
            "code→list gap {} should be ~paragraph_spacing ({}), not 2x ({})",
            gap,
            expected,
            expected * 2.0
        );
    }

    #[test]
    fn bq_to_para_not_double_spaced() {
        let (src, doc) = make_doc("> quote\n\nnext paragraph");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
        assert!(laid_out.blocks.len() >= 2);
        let bq_bottom = laid_out.blocks[0].rect.y + laid_out.blocks[0].rect.h;
        let next_top = laid_out.blocks[1].rect.y;
        let gap = next_top - bq_bottom;
        let expected = style.paragraph_spacing;
        assert!(
            (gap - expected).abs() < 1.0,
            "bq→para gap {} should be ~paragraph_spacing ({}), not 2x ({})",
            gap,
            expected,
            expected * 2.0
        );
    }

    #[test]
    fn hr_to_list_not_double_spaced() {
        let (src, doc) = make_doc("---\n\n- item");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
        let hr = laid_out
            .blocks
            .iter()
            .find(|b| matches!(&b.kind, LaidOutBlockKind::HorizontalRule))
            .unwrap();
        let list_item = laid_out
            .blocks
            .iter()
            .find(|b| matches!(&b.kind, LaidOutBlockKind::ListItem { .. }))
            .unwrap();
        let block_gap = list_item.rect.y - (hr.rect.y + hr.rect.h);
        assert!(
            block_gap.abs() < 1.0,
            "block gap between HR and list should be ~0, got {}",
            block_gap
        );
        let visual_gap = list_item.rect.y - (hr.rect.y + style.rule_spacing + style.rule_thickness);
        assert!(
            visual_gap >= style.rule_spacing - 1.0,
            "visual gap from HR rule to list {} should be >= rule_spacing ({})",
            visual_gap,
            style.rule_spacing
        );
        assert!(
            visual_gap < style.rule_spacing + style.paragraph_spacing,
            "visual gap {} should NOT include extra paragraph_spacing",
            visual_gap
        );
    }

    #[test]
    fn heading_hr_heading_no_stale_heading_flag() {
        let (src, doc) = make_doc("## First\n\n---\n\n## Second");
        let style = default_style();
        let laid_out =
            layout_doc(&doc.blocks, &style, 400.0, &core::document::StringDocView::new(src));
        let hr = laid_out
            .blocks
            .iter()
            .find(|b| matches!(&b.kind, LaidOutBlockKind::HorizontalRule))
            .unwrap();
        let second_heading = laid_out
            .blocks
            .iter()
            .find(|b| {
                matches!(&b.kind, LaidOutBlockKind::Text { .. }) && b.rect.y > hr.rect.y + hr.rect.h
            })
            .unwrap();
        let visual_gap =
            second_heading.rect.y - (hr.rect.y + style.rule_spacing + style.rule_thickness);
        let expected = style.heading_spacing_top * 0.8;
        assert!(
            (visual_gap - expected).abs() < 1.5,
            "visual gap from HR rule line to H2 {} should be ~heading_spacing_top ({})",
            visual_gap,
            expected
        );
    }
}
