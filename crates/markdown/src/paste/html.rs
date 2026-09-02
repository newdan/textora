use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Node};
use url::Url;

use super::{
    HeadingLevel, InlineSemantic, ListKind, RichBlock, RichDocument, RichInline, VisibleSegment,
};

pub(crate) const MAX_HTML_NESTING_DEPTH: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SemanticMarkup {
    Absent,
    Present,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HtmlPasteError {
    NestingDepthExceeded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SupportedStyleProperty {
    FontWeight,
    FontStyle,
    TextDecoration,
    Display,
    Visibility,
}

struct TableRowSource<'a> {
    element: ElementRef<'a>,
    group_semantics: Vec<InlineSemantic>,
}

struct CascadedStyleDeclaration {
    property: SupportedStyleProperty,
    value: String,
    important: bool,
}

#[derive(Clone, Copy)]
enum RawTextElement {
    Script,
    Style,
    Textarea,
    Title,
    Xmp,
    Iframe,
    NoEmbed,
    NoFrames,
    Plaintext,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TagQuote {
    None,
    Single,
    Double,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InlineFormattingTag {
    Strong,
    Bold,
    Emphasis,
    Italic,
    Strikethrough,
    Strike,
    Delete,
    Link,
    Code,
}

#[derive(Clone, Copy)]
enum SourceFormattingState {
    Open { tag: InlineFormattingTag, paragraph_scope: Option<usize> },
    SyntheticClosed { tag: InlineFormattingTag },
}

#[derive(Default)]
struct FragmentNormalizer {
    current_paragraph_scope: Option<usize>,
    next_paragraph_scope: usize,
    source_formatting: Vec<SourceFormattingState>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceTagDisposition {
    Emit,
    Suppress,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HtmlConversion {
    pub document: RichDocument,
    pub semantic_markup: SemanticMarkup,
}

pub(crate) fn parse_html(
    html: &str,
    source_url: Option<&str>,
) -> Result<HtmlConversion, HtmlPasteError> {
    let normalized_html = normalize_malformed_fragment(html)?;
    let fragment = Html::parse_fragment(&normalized_html);
    let root = fragment.root_element();
    ensure_dom_depth_within_limit(*root)?;
    let base_url = source_url.and_then(|source| Url::parse(source).ok());
    let blocks = parse_block_children(*root, base_url.as_ref());
    let document = RichDocument::new(blocks);
    let semantic_markup = document_semantic_markup(&document);
    Ok(HtmlConversion { document, semantic_markup })
}

fn ensure_dom_depth_within_limit(root: NodeRef<'_, Node>) -> Result<(), HtmlPasteError> {
    let mut pending_nodes = vec![(root, 0_usize)];
    while let Some((node, depth)) = pending_nodes.pop() {
        if depth > MAX_HTML_NESTING_DEPTH {
            return Err(HtmlPasteError::NestingDepthExceeded);
        }
        pending_nodes.extend(node.children().map(|child| {
            let child_depth = depth + usize::from(child.value().is_element());
            (child, child_depth)
        }));
    }
    Ok(())
}

fn parse_block_children(node: NodeRef<'_, Node>, base_url: Option<&Url>) -> Vec<RichBlock> {
    parse_block_nodes(node.children(), base_url)
}

fn parse_block_nodes<'a>(
    nodes: impl IntoIterator<Item = NodeRef<'a, Node>>,
    base_url: Option<&Url>,
) -> Vec<RichBlock> {
    let mut blocks = Vec::new();
    let mut inline_run = Vec::new();
    for child in nodes {
        if ElementRef::wrap(child).is_some_and(element_is_hidden) {
            continue;
        }
        if is_block_node(child) {
            flush_inline_run(&mut inline_run, &mut blocks, base_url);
            blocks.extend(parse_block_node(child, base_url));
        } else {
            inline_run.push(child);
        }
    }
    flush_inline_run(&mut inline_run, &mut blocks, base_url);
    blocks
}

fn flush_inline_run(
    inline_run: &mut Vec<NodeRef<'_, Node>>,
    blocks: &mut Vec<RichBlock>,
    base_url: Option<&Url>,
) {
    let mut content =
        inline_run.drain(..).flat_map(|node| parse_inline_node(node, base_url)).collect::<Vec<_>>();
    normalize_flow_content(&mut content);
    if inline_content_is_empty(&content) {
        return;
    }
    blocks.push(RichBlock::Paragraph(content));
}

fn parse_block_node(node: NodeRef<'_, Node>, base_url: Option<&Url>) -> Vec<RichBlock> {
    let Some(element) = ElementRef::wrap(node) else {
        return paragraph_for_inline_node(node, base_url);
    };
    if element_is_hidden(element) {
        return Vec::new();
    }
    let blocks = match element.value().name() {
        "h1" => heading(element, HeadingLevel::H1, base_url),
        "h2" => heading(element, HeadingLevel::H2, base_url),
        "h3" => heading(element, HeadingLevel::H3, base_url),
        "h4" => heading(element, HeadingLevel::H4, base_url),
        "h5" => heading(element, HeadingLevel::H5, base_url),
        "h6" => heading(element, HeadingLevel::H6, base_url),
        "p" => paragraph_for_element(element, base_url),
        "blockquote" => vec![RichBlock::BlockQuote(parse_block_children(node, base_url))],
        "ul" => parse_list(element, ListKind::Unordered, base_url),
        "ol" => parse_list(element, ordered_list_kind(element), base_url),
        "pre" => vec![parse_code_block(element)],
        "table" => parse_table(element, base_url),
        "hr" => vec![RichBlock::HorizontalRule],
        _ => parse_block_children(node, base_url),
    };
    apply_element_semantics_to_blocks(element, blocks)
}

fn heading(element: ElementRef<'_>, level: HeadingLevel, base_url: Option<&Url>) -> Vec<RichBlock> {
    let mut content = parse_inline_children(*element, base_url);
    normalize_flow_content(&mut content);
    vec![RichBlock::Heading { level, content }]
}

fn paragraph_for_element(element: ElementRef<'_>, base_url: Option<&Url>) -> Vec<RichBlock> {
    let mut content = parse_inline_children(*element, base_url);
    normalize_flow_content(&mut content);
    if inline_content_is_empty(&content) { Vec::new() } else { vec![RichBlock::Paragraph(content)] }
}

fn paragraph_for_inline_node(node: NodeRef<'_, Node>, base_url: Option<&Url>) -> Vec<RichBlock> {
    let mut content = parse_inline_node(node, base_url);
    normalize_flow_content(&mut content);
    if inline_content_is_empty(&content) { Vec::new() } else { vec![RichBlock::Paragraph(content)] }
}

fn parse_inline_children(node: NodeRef<'_, Node>, base_url: Option<&Url>) -> Vec<RichInline> {
    node.children().flat_map(|child| parse_inline_node(child, base_url)).collect()
}

fn parse_inline_node(node: NodeRef<'_, Node>, base_url: Option<&Url>) -> Vec<RichInline> {
    if let Node::Text(text) = node.value() {
        return vec![RichInline::Text(collapse_html_whitespace(text))];
    }
    let Some(element) = ElementRef::wrap(node) else {
        return Vec::new();
    };
    if element_is_hidden(element) {
        return Vec::new();
    }
    match element.value().name() {
        "br" => vec![RichInline::LineBreak],
        "code" => {
            let code = vec![RichInline::InlineCode(visible_text(*element))];
            apply_inline_semantics(element, code)
        }
        "a" => parse_link(element, base_url),
        "img" => parse_image(element, base_url),
        _ => parse_styled_inline(element, base_url),
    }
}

fn parse_styled_inline(element: ElementRef<'_>, base_url: Option<&Url>) -> Vec<RichInline> {
    let children = parse_inline_children(*element, base_url);
    apply_inline_semantics(element, children)
}

fn apply_inline_semantics(element: ElementRef<'_>, children: Vec<RichInline>) -> Vec<RichInline> {
    let semantics = inline_semantics(element);
    wrap_inline_semantics(&semantics, children)
}

fn wrap_inline_semantics(
    semantics: &[InlineSemantic],
    children: Vec<RichInline>,
) -> Vec<RichInline> {
    if semantics.is_empty() {
        return children;
    }
    let mut wrapped_siblings = Vec::with_capacity(children.len());
    for child in children {
        let wrapped = wrap_single_inline_semantics(semantics, child);
        for inline in wrapped {
            append_merging_semantic_sibling(&mut wrapped_siblings, inline);
        }
    }
    wrapped_siblings
}

fn wrap_single_inline_semantics(
    semantics: &[InlineSemantic],
    child: RichInline,
) -> Vec<RichInline> {
    let mut canonical_semantics = semantics.to_vec();
    let unwrapped_children = peel_outer_inline_semantics(vec![child], &mut canonical_semantics);
    canonical_semantics.sort_by_key(semantic_order);
    canonical_semantics.dedup();
    canonical_semantics
        .iter()
        .rev()
        .fold(unwrapped_children, |nested, semantic| vec![wrap_inline(*semantic, nested)])
}

fn append_merging_semantic_sibling(siblings: &mut Vec<RichInline>, inline: RichInline) {
    match (siblings.last_mut(), inline) {
        (Some(RichInline::Strong(existing)), RichInline::Strong(mut children))
        | (Some(RichInline::Emphasis(existing)), RichInline::Emphasis(mut children))
        | (Some(RichInline::Strikethrough(existing)), RichInline::Strikethrough(mut children)) => {
            existing.append(&mut children)
        }
        (_, inline) => siblings.push(inline),
    }
}

fn peel_outer_inline_semantics(
    mut children: Vec<RichInline>,
    semantics: &mut Vec<InlineSemantic>,
) -> Vec<RichInline> {
    loop {
        if children.len() != 1 {
            return children;
        }
        let outer = children.pop().expect("one outer inline was checked");
        children = match outer {
            RichInline::Strong(nested) => peel_semantic(InlineSemantic::Strong, nested, semantics),
            RichInline::Emphasis(nested) => {
                peel_semantic(InlineSemantic::Emphasis, nested, semantics)
            }
            RichInline::Strikethrough(nested) => {
                peel_semantic(InlineSemantic::Strikethrough, nested, semantics)
            }
            outer => return vec![outer],
        };
    }
}

fn peel_semantic(
    semantic: InlineSemantic,
    children: Vec<RichInline>,
    semantics: &mut Vec<InlineSemantic>,
) -> Vec<RichInline> {
    semantics.push(semantic);
    children
}

fn wrap_inline(semantic: InlineSemantic, children: Vec<RichInline>) -> RichInline {
    match semantic {
        InlineSemantic::Strong => RichInline::Strong(children),
        InlineSemantic::Emphasis => RichInline::Emphasis(children),
        InlineSemantic::Strikethrough => RichInline::Strikethrough(children),
    }
}

fn parse_link(element: ElementRef<'_>, base_url: Option<&Url>) -> Vec<RichInline> {
    let children = parse_inline_children(*element, base_url);
    let children = apply_inline_semantics(element, children);
    let Some(destination) = element.attr("href").and_then(|raw| resolve_destination(raw, base_url))
    else {
        return children;
    };
    if !matches!(destination.scheme(), "http" | "https" | "mailto") {
        return children;
    }
    vec![RichInline::Link {
        destination: destination.into(),
        title: nonempty_attribute(element, "title"),
        children,
    }]
}

fn parse_image(element: ElementRef<'_>, base_url: Option<&Url>) -> Vec<RichInline> {
    let alt = element.attr("alt").unwrap_or_default().to_owned();
    let destination = element.attr("src").and_then(|raw| resolve_destination(raw, base_url));
    if let Some(destination) = destination.filter(|url| matches!(url.scheme(), "http" | "https")) {
        return vec![RichInline::RemoteImage {
            destination: destination.into(),
            title: nonempty_attribute(element, "title"),
            alt,
        }];
    }
    (!alt.is_empty()).then_some(RichInline::Text(alt)).into_iter().collect()
}

fn parse_list(element: ElementRef<'_>, kind: ListKind, base_url: Option<&Url>) -> Vec<RichBlock> {
    let mut blocks = Vec::new();
    let mut items = Vec::new();
    let mut pending_non_items = Vec::new();
    let mut emitted_items = 0_u64;
    for child in element.children() {
        if ElementRef::wrap(child).is_some_and(element_is_hidden) {
            continue;
        }
        if ElementRef::wrap(child).is_some_and(|child| child.value().name() == "li") {
            flush_list_non_items(
                &mut pending_non_items,
                &mut items,
                &mut blocks,
                kind,
                &mut emitted_items,
                base_url,
            );
            let item = ElementRef::wrap(child).expect("list item element was checked");
            let item_blocks = parse_block_children(child, base_url);
            items.push(apply_element_semantics_to_blocks(item, item_blocks));
            continue;
        }
        pending_non_items.push(child);
    }
    flush_list_non_items(
        &mut pending_non_items,
        &mut items,
        &mut blocks,
        kind,
        &mut emitted_items,
        base_url,
    );
    flush_list_items(&mut items, &mut blocks, kind, &mut emitted_items);
    blocks
}

fn flush_list_non_items<'a>(
    pending: &mut Vec<NodeRef<'a, Node>>,
    items: &mut Vec<Vec<RichBlock>>,
    blocks: &mut Vec<RichBlock>,
    kind: ListKind,
    emitted_items: &mut u64,
    base_url: Option<&Url>,
) {
    let mut non_item_blocks = parse_block_nodes(pending.drain(..), base_url);
    if !blocks_have_visible_content(&non_item_blocks) {
        return;
    }
    flush_list_items(items, blocks, kind, emitted_items);
    blocks.append(&mut non_item_blocks);
}

fn flush_list_items(
    items: &mut Vec<Vec<RichBlock>>,
    blocks: &mut Vec<RichBlock>,
    kind: ListKind,
    emitted_items: &mut u64,
) {
    if items.is_empty() {
        return;
    }
    let segment_kind = match kind {
        ListKind::Unordered => ListKind::Unordered,
        ListKind::Ordered { start } => {
            ListKind::Ordered { start: start.saturating_add(*emitted_items) }
        }
    };
    *emitted_items += items.len() as u64;
    blocks.push(RichBlock::List { kind: segment_kind, items: std::mem::take(items) });
}

fn ordered_list_kind(element: ElementRef<'_>) -> ListKind {
    let start = element.attr("start").and_then(|value| value.parse().ok()).unwrap_or(1);
    ListKind::Ordered { start }
}

fn parse_code_block(element: ElementRef<'_>) -> RichBlock {
    let code = element
        .child_elements()
        .find(|child| child.value().name() == "code" && !element_is_hidden(*child));
    let language = code.and_then(code_language);
    let text = visible_text(*element);
    RichBlock::CodeBlock { language, text }
}

fn code_language(element: ElementRef<'_>) -> Option<String> {
    element.attr("class")?.split_ascii_whitespace().find_map(|class_name| {
        class_name
            .strip_prefix("language-")
            .filter(|language| !language.is_empty())
            .map(str::to_owned)
    })
}

fn parse_table(element: ElementRef<'_>, base_url: Option<&Url>) -> Vec<RichBlock> {
    let (mut blocks, trailing_captions) = parse_table_captions(element, base_url);
    let table_rows = collect_table_rows(element);
    let mut parsed_rows = table_rows
        .into_iter()
        .map(|row| parse_table_row(row, base_url))
        .filter(|row| !row.is_empty());
    let Some(header) = parsed_rows.next() else {
        blocks.extend(trailing_captions);
        return blocks;
    };
    blocks.push(RichBlock::Table { header, rows: parsed_rows.collect() });
    blocks.extend(trailing_captions);
    blocks
}

fn parse_table_captions(
    element: ElementRef<'_>,
    base_url: Option<&Url>,
) -> (Vec<RichBlock>, Vec<RichBlock>) {
    let mut leading = Vec::new();
    let mut trailing = Vec::new();
    let mut encountered_rows = false;
    for child in element.child_elements() {
        if element_is_hidden(child) {
            continue;
        }
        match child.value().name() {
            "tr" | "thead" | "tbody" | "tfoot" => encountered_rows = true,
            "caption" => {
                let caption_blocks = paragraph_for_element(child, base_url);
                let caption_blocks = apply_element_semantics_to_blocks(child, caption_blocks);
                if encountered_rows {
                    trailing.extend(caption_blocks);
                } else {
                    leading.extend(caption_blocks);
                }
            }
            _ => {}
        }
    }
    (leading, trailing)
}

fn collect_table_rows(element: ElementRef<'_>) -> Vec<TableRowSource<'_>> {
    element
        .child_elements()
        .flat_map(|child| match child.value().name() {
            _ if element_is_hidden(child) => Vec::new(),
            "tr" => vec![TableRowSource { element: child, group_semantics: Vec::new() }],
            "thead" | "tbody" | "tfoot" => {
                let group_semantics = inline_semantics(child);
                child
                    .child_elements()
                    .filter(|row| row.value().name() == "tr" && !element_is_hidden(*row))
                    .map(|row| TableRowSource {
                        element: row,
                        group_semantics: group_semantics.clone(),
                    })
                    .collect()
            }
            _ => Vec::new(),
        })
        .collect()
}

fn parse_table_row(row: TableRowSource<'_>, base_url: Option<&Url>) -> Vec<Vec<RichInline>> {
    let row_semantics = inline_semantics(row.element);
    row.element
        .child_elements()
        .filter(|cell| matches!(cell.value().name(), "th" | "td") && !element_is_hidden(*cell))
        .map(|cell| {
            let content = parse_table_cell_content(cell, base_url);
            let semantics = canonical_table_cell_semantics(
                &row.group_semantics,
                &row_semantics,
                &inline_semantics(cell),
            );
            wrap_inline_semantics(&semantics, content)
        })
        .collect()
}

fn parse_table_cell_content(element: ElementRef<'_>, base_url: Option<&Url>) -> Vec<RichInline> {
    let blocks = parse_block_children(*element, base_url);
    let mut content = Vec::new();
    for block in blocks {
        let block_content = table_block_content(block);
        if block_content.is_empty() {
            continue;
        }
        if !content.is_empty() {
            content.push(RichInline::LineBreak);
        }
        content.extend(block_content);
    }
    content
}

fn table_block_content(block: RichBlock) -> Vec<RichInline> {
    match block {
        RichBlock::Heading { content, .. } | RichBlock::Paragraph(content) => content,
        block => visible_segments_to_inlines(RichDocument::new(vec![block]).visible_segments()),
    }
}

fn visible_segments_to_inlines(segments: Vec<VisibleSegment>) -> Vec<RichInline> {
    let mut inlines = Vec::new();
    let mut wrote_segment = false;
    for segment in segments {
        if segment.text.is_empty() {
            continue;
        }
        if wrote_segment {
            inlines.push(RichInline::LineBreak);
        }
        inlines.extend(visible_text_to_inlines(&segment.text));
        wrote_segment = true;
    }
    inlines
}

fn visible_text_to_inlines(text: &str) -> Vec<RichInline> {
    let mut inlines = Vec::new();
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            inlines.push(RichInline::LineBreak);
        }
        if !line.is_empty() {
            inlines.push(RichInline::Text(line.to_owned()));
        }
    }
    inlines
}

fn canonical_table_cell_semantics(
    group: &[InlineSemantic],
    row: &[InlineSemantic],
    cell: &[InlineSemantic],
) -> Vec<InlineSemantic> {
    let mut semantics = Vec::with_capacity(group.len() + row.len() + cell.len());
    semantics.extend_from_slice(group);
    semantics.extend_from_slice(row);
    semantics.extend_from_slice(cell);
    semantics.sort_by_key(semantic_order);
    semantics.dedup();
    semantics
}

fn apply_element_semantics_to_blocks(
    element: ElementRef<'_>,
    mut blocks: Vec<RichBlock>,
) -> Vec<RichBlock> {
    let semantics = inline_semantics(element);
    if semantics.is_empty() {
        return blocks;
    }
    for block in &mut blocks {
        apply_semantics_to_block(block, &semantics);
    }
    blocks
}

fn apply_semantics_to_block(block: &mut RichBlock, semantics: &[InlineSemantic]) {
    match block {
        RichBlock::Heading { content, .. } | RichBlock::Paragraph(content) => {
            *content = wrap_inline_semantics(semantics, std::mem::take(content));
        }
        RichBlock::BlockQuote(blocks) => apply_semantics_to_blocks(blocks, semantics),
        RichBlock::List { items, .. } => {
            for blocks in items {
                apply_semantics_to_blocks(blocks, semantics);
            }
        }
        RichBlock::Table { header, rows } => {
            apply_semantics_to_cells(header, semantics);
            rows.iter_mut().for_each(|row| apply_semantics_to_cells(row, semantics));
        }
        RichBlock::CodeBlock { .. } | RichBlock::HorizontalRule => {}
    }
}

fn apply_semantics_to_blocks(blocks: &mut [RichBlock], semantics: &[InlineSemantic]) {
    blocks.iter_mut().for_each(|block| apply_semantics_to_block(block, semantics));
}

fn apply_semantics_to_cells(cells: &mut [Vec<RichInline>], semantics: &[InlineSemantic]) {
    for content in cells {
        *content = wrap_inline_semantics(semantics, std::mem::take(content));
    }
}

fn inline_semantics(element: ElementRef<'_>) -> Vec<InlineSemantic> {
    let mut semantics = Vec::new();
    match element.value().name() {
        "strong" | "b" => semantics.push(InlineSemantic::Strong),
        "em" | "i" => semantics.push(InlineSemantic::Emphasis),
        "s" | "strike" | "del" => semantics.push(InlineSemantic::Strikethrough),
        _ => {}
    }
    for declaration in own_style_declarations(element) {
        let semantic = style_semantic(declaration.property, &declaration.value);
        if let Some(semantic) = semantic.filter(|candidate| !semantics.contains(candidate)) {
            semantics.push(semantic);
        }
    }
    semantics.sort_by_key(semantic_order);
    semantics
}

fn style_semantic(property: SupportedStyleProperty, value: &str) -> Option<InlineSemantic> {
    match property {
        SupportedStyleProperty::FontWeight if font_weight_is_strong(value) => {
            Some(InlineSemantic::Strong)
        }
        SupportedStyleProperty::FontStyle
            if matches!(first_css_token(value), "italic" | "oblique") =>
        {
            Some(InlineSemantic::Emphasis)
        }
        SupportedStyleProperty::TextDecoration
            if value.split_ascii_whitespace().any(|token| token == "line-through") =>
        {
            Some(InlineSemantic::Strikethrough)
        }
        _ => None,
    }
}

fn font_weight_is_strong(value: &str) -> bool {
    let token = first_css_token(value);
    matches!(token, "bold" | "bolder") || token.parse::<u16>().is_ok_and(|weight| weight >= 600)
}

fn semantic_order(semantic: &InlineSemantic) -> u8 {
    match semantic {
        InlineSemantic::Strong => 0,
        InlineSemantic::Emphasis => 1,
        InlineSemantic::Strikethrough => 2,
    }
}

fn resolve_destination(raw: &str, base_url: Option<&Url>) -> Option<Url> {
    let destination = raw.trim();
    if destination.is_empty() {
        return None;
    }
    Url::parse(destination).ok().or_else(|| base_url?.join(destination).ok())
}

fn element_is_hidden(element: ElementRef<'_>) -> bool {
    if matches!(
        element.value().name(),
        "head" | "title" | "script" | "style" | "template" | "iframe" | "noembed" | "noframes"
    ) {
        return true;
    }
    if element.attr("hidden").is_some()
        || element
            .attr("aria-hidden")
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
    {
        return true;
    }
    own_style_declarations(element).into_iter().any(|declaration| {
        matches!(
            (declaration.property, first_css_token(&declaration.value)),
            (SupportedStyleProperty::Display, "none")
                | (SupportedStyleProperty::Visibility, "hidden")
        )
    })
}

fn own_style_declarations(element: ElementRef<'_>) -> Vec<CascadedStyleDeclaration> {
    let Some(style) = element.attr("style") else {
        return Vec::new();
    };
    let mut cascaded = Vec::<CascadedStyleDeclaration>::new();
    for raw_declaration in style.split(';') {
        let Some(declaration) = parse_style_declaration(raw_declaration) else {
            continue;
        };
        if let Some(existing) =
            cascaded.iter_mut().find(|current| current.property == declaration.property)
        {
            if !existing.important || declaration.important {
                *existing = declaration;
            }
        } else {
            cascaded.push(declaration);
        }
    }
    cascaded
}

fn parse_style_declaration(raw: &str) -> Option<CascadedStyleDeclaration> {
    let (property, value) = raw.split_once(':')?;
    let property = supported_style_property(property.trim())?;
    let lowercase_value = value.trim().to_ascii_lowercase();
    let (value, important) = split_important_value(&lowercase_value);
    Some(CascadedStyleDeclaration { property, value, important })
}

fn split_important_value(value: &str) -> (String, bool) {
    let Some(marker_start) = value.rfind('!') else {
        return (value.trim().to_owned(), false);
    };
    if !value[marker_start + 1..].trim().eq_ignore_ascii_case("important") {
        return (value.trim().to_owned(), false);
    }
    (value[..marker_start].trim().to_owned(), true)
}

fn supported_style_property(property: &str) -> Option<SupportedStyleProperty> {
    match property.to_ascii_lowercase().as_str() {
        "font-weight" => Some(SupportedStyleProperty::FontWeight),
        "font-style" => Some(SupportedStyleProperty::FontStyle),
        "text-decoration" => Some(SupportedStyleProperty::TextDecoration),
        "display" => Some(SupportedStyleProperty::Display),
        "visibility" => Some(SupportedStyleProperty::Visibility),
        _ => None,
    }
}

fn first_css_token(value: &str) -> &str {
    value.split_ascii_whitespace().next().unwrap_or_default()
}

fn visible_text(node: NodeRef<'_, Node>) -> String {
    let mut output = String::new();
    append_visible_text(node, &mut output);
    output
}

fn append_visible_text(node: NodeRef<'_, Node>, output: &mut String) {
    if let Node::Text(text) = node.value() {
        output.push_str(text);
        return;
    }
    if let Some(element) = ElementRef::wrap(node) {
        if element_is_hidden(element) {
            return;
        }
        if element.value().name() == "br" {
            output.push('\n');
            return;
        }
    }
    for child in node.children() {
        append_visible_text(child, output);
    }
}

fn nonempty_attribute(element: ElementRef<'_>, name: &str) -> Option<String> {
    element.attr(name).map(str::trim).filter(|value| !value.is_empty()).map(str::to_owned)
}

fn is_block_node(node: NodeRef<'_, Node>) -> bool {
    let Some(element) = ElementRef::wrap(node) else {
        return false;
    };
    known_block_tag(element.value().name()) || node.children().any(is_block_node)
}

fn known_block_tag(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "div"
            | "dl"
            | "fieldset"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "ul"
            | "xmp"
            | "plaintext"
    )
}

fn collapse_html_whitespace(text: &str) -> String {
    let starts_with_space = text.chars().next().is_some_and(char::is_whitespace);
    let ends_with_space = text.chars().next_back().is_some_and(char::is_whitespace);
    let mut collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if starts_with_space {
        collapsed.insert(0, ' ');
    }
    if ends_with_space && !collapsed.ends_with(' ') {
        collapsed.push(' ');
    }
    collapsed
}

fn normalize_flow_content(content: &mut Vec<RichInline>) {
    merge_adjacent_text(content);
    if let Some(RichInline::Text(text)) = content.first_mut() {
        *text = text.trim_start().to_owned();
    }
    if let Some(RichInline::Text(text)) = content.last_mut() {
        *text = text.trim_end().to_owned();
    }
    content.retain(|inline| !matches!(inline, RichInline::Text(text) if text.is_empty()));
}

fn merge_adjacent_text(content: &mut Vec<RichInline>) {
    let mut merged = Vec::with_capacity(content.len());
    for inline in content.drain(..) {
        match (merged.last_mut(), inline) {
            (Some(RichInline::Text(existing)), RichInline::Text(text)) => existing.push_str(&text),
            (_, RichInline::Text(text)) if text.is_empty() => {}
            (_, inline) => merged.push(inline),
        }
    }
    for inline in &mut merged {
        if let RichInline::Text(text) = inline {
            *text = collapse_html_whitespace(text);
        }
    }
    *content = merged;
}

fn normalize_malformed_fragment(html: &str) -> Result<String, HtmlPasteError> {
    let lowercase_html = html.to_ascii_lowercase();
    let mut normalized = String::with_capacity(html.len());
    let mut normalizer = FragmentNormalizer::default();
    let mut cursor = 0;
    while cursor < html.len() {
        let Some(tag_start) = find_next_tag_start(html, cursor) else {
            normalized.push_str(&html[cursor..]);
            break;
        };
        normalized.push_str(&html[cursor..tag_start]);
        if html[tag_start..].starts_with("<!--") {
            cursor = copy_html_comment(html, tag_start, &mut normalized);
            continue;
        }
        let Some(tag_end) = find_tag_end(html, tag_start) else {
            normalized.push_str(&html[tag_start..]);
            break;
        };
        let source_tag = &html[tag_start..=tag_end];
        let Some((name, closing)) = source_tag_name(source_tag) else {
            normalized.push_str(source_tag);
            cursor = tag_end + 1;
            continue;
        };
        if name == "head" {
            normalized.push_str(if closing { "</template>" } else { "<template>" });
            cursor = tag_end + 1;
            continue;
        }
        normalizer.prepare_for_tag(&name, closing, &mut normalized);
        if !closing && raw_text_element(&name).is_some() {
            cursor = copy_raw_text_element(
                html,
                &lowercase_html,
                tag_start,
                tag_end,
                &name,
                &mut normalized,
            );
            continue;
        }
        if normalizer.record_formatting_tag(&name, closing)? == SourceTagDisposition::Emit {
            normalized.push_str(source_tag);
        }
        cursor = tag_end + 1;
    }
    Ok(normalized)
}

fn find_next_tag_start(html: &str, cursor: usize) -> Option<usize> {
    html[cursor..].find('<').map(|offset| cursor + offset)
}

fn copy_html_comment(html: &str, tag_start: usize, normalized: &mut String) -> usize {
    let comment_end = find_html_comment_end(html, tag_start);
    normalized.push_str(&html[tag_start..comment_end]);
    comment_end
}

fn find_html_comment_end(html: &str, tag_start: usize) -> usize {
    const STANDARD_COMMENT_END: &[u8] = b"-->";
    const BANG_COMMENT_END: &[u8] = b"--!>";
    let bytes = html.as_bytes();
    let content_start = tag_start + 4;
    let content = &bytes[content_start..];
    if content.starts_with(b">") {
        return content_start + 1;
    }
    if content.starts_with(b"->") {
        return content_start + 2;
    }
    for cursor in content_start..bytes.len() {
        let remaining = &bytes[cursor..];
        if remaining.starts_with(STANDARD_COMMENT_END) {
            return cursor + STANDARD_COMMENT_END.len();
        }
        if remaining.starts_with(BANG_COMMENT_END) {
            return cursor + BANG_COMMENT_END.len();
        }
    }
    bytes.len()
}

fn find_tag_end(html: &str, tag_start: usize) -> Option<usize> {
    let mut quote = TagQuote::None;
    for (offset, byte) in html.as_bytes()[tag_start + 1..].iter().enumerate() {
        quote = match (quote, byte) {
            (TagQuote::None, b'\'') => TagQuote::Single,
            (TagQuote::None, b'"') => TagQuote::Double,
            (TagQuote::Single, b'\'') | (TagQuote::Double, b'"') => TagQuote::None,
            (current, _) => current,
        };
        if quote == TagQuote::None && *byte == b'>' {
            return Some(tag_start + 1 + offset);
        }
    }
    None
}

fn tag_is_paragraph_boundary(name: &str) -> bool {
    known_block_tag(name) || matches!(name, "li" | "dt" | "dd" | "caption" | "tr" | "th" | "td")
}

fn tag_starts_paragraph_scope(name: &str) -> bool {
    matches!(name, "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

fn source_tag_name(source: &str) -> Option<(String, bool)> {
    let bytes = source.as_bytes();
    let closing = bytes.get(1) == Some(&b'/');
    let mut cursor = 1 + usize::from(closing);
    if !bytes.get(cursor).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let name_start = cursor;
    while bytes
        .get(cursor)
        .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'/' | b'>'))
    {
        cursor += 1;
    }
    (cursor > name_start).then(|| (source[name_start..cursor].to_ascii_lowercase(), closing))
}

fn raw_text_element(name: &str) -> Option<RawTextElement> {
    match name {
        "script" => Some(RawTextElement::Script),
        "style" => Some(RawTextElement::Style),
        "textarea" => Some(RawTextElement::Textarea),
        "title" => Some(RawTextElement::Title),
        "xmp" => Some(RawTextElement::Xmp),
        "iframe" => Some(RawTextElement::Iframe),
        "noembed" => Some(RawTextElement::NoEmbed),
        "noframes" => Some(RawTextElement::NoFrames),
        "plaintext" => Some(RawTextElement::Plaintext),
        _ => None,
    }
}

fn copy_raw_text_element(
    html: &str,
    lowercase_html: &str,
    tag_start: usize,
    tag_end: usize,
    name: &str,
    normalized: &mut String,
) -> usize {
    let element = raw_text_element(name).expect("caller checked the raw-text element name");
    let Some(closing_start) = find_raw_text_closing(lowercase_html, tag_end + 1, element) else {
        normalized.push_str(&html[tag_start..]);
        return html.len();
    };
    let Some(closing_end) = find_tag_end(html, closing_start) else {
        normalized.push_str(&html[tag_start..]);
        return html.len();
    };
    normalized.push_str(&html[tag_start..=closing_end]);
    closing_end + 1
}

fn find_raw_text_closing(
    lowercase_html: &str,
    cursor: usize,
    element: RawTextElement,
) -> Option<usize> {
    if matches!(element, RawTextElement::Plaintext) {
        return None;
    }
    let closing_prefix = format!("</{}", element.name());
    let mut search_start = cursor;
    while let Some(offset) = lowercase_html[search_start..].find(&closing_prefix) {
        let candidate = search_start + offset;
        let boundary = lowercase_html.as_bytes().get(candidate + closing_prefix.len());
        if boundary.is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'>' | b'/')) {
            return Some(candidate);
        }
        search_start = candidate + closing_prefix.len();
    }
    None
}

impl RawTextElement {
    fn name(self) -> &'static str {
        match self {
            Self::Script => "script",
            Self::Style => "style",
            Self::Textarea => "textarea",
            Self::Title => "title",
            Self::Xmp => "xmp",
            Self::Iframe => "iframe",
            Self::NoEmbed => "noembed",
            Self::NoFrames => "noframes",
            Self::Plaintext => "plaintext",
        }
    }
}

impl FragmentNormalizer {
    fn prepare_for_tag(&mut self, name: &str, closing: bool, normalized: &mut String) {
        if !tag_is_paragraph_boundary(name) {
            return;
        }
        self.close_paragraph_formatting(normalized);
        self.current_paragraph_scope = None;
        if !closing && tag_starts_paragraph_scope(name) {
            self.next_paragraph_scope += 1;
            self.current_paragraph_scope = Some(self.next_paragraph_scope);
        }
    }

    fn close_paragraph_formatting(&mut self, normalized: &mut String) {
        let Some(scope) = self.current_paragraph_scope else {
            return;
        };
        let mut closing_tags = Vec::new();
        for state in &mut self.source_formatting {
            let SourceFormattingState::Open { tag, paragraph_scope } = *state else {
                continue;
            };
            if paragraph_scope != Some(scope) {
                continue;
            }
            closing_tags.push(tag);
            *state = SourceFormattingState::SyntheticClosed { tag };
        }
        for tag in closing_tags.into_iter().rev() {
            normalized.push_str("</");
            normalized.push_str(tag.name());
            normalized.push('>');
        }
    }

    fn record_formatting_tag(
        &mut self,
        name: &str,
        closing: bool,
    ) -> Result<SourceTagDisposition, HtmlPasteError> {
        let Some(tag) = InlineFormattingTag::from_name(name) else {
            return Ok(SourceTagDisposition::Emit);
        };
        if closing {
            return Ok(self.close_source_formatting(tag));
        } else {
            if self.source_formatting.len() >= MAX_HTML_NESTING_DEPTH {
                return Err(HtmlPasteError::NestingDepthExceeded);
            }
            self.source_formatting.push(SourceFormattingState::Open {
                tag,
                paragraph_scope: self.current_paragraph_scope,
            });
        }
        Ok(SourceTagDisposition::Emit)
    }

    fn close_source_formatting(&mut self, tag: InlineFormattingTag) -> SourceTagDisposition {
        let Some(index) = self.source_formatting.iter().rposition(|state| state.tag() == tag)
        else {
            return SourceTagDisposition::Emit;
        };
        match self.source_formatting.remove(index) {
            SourceFormattingState::Open { .. } => SourceTagDisposition::Emit,
            SourceFormattingState::SyntheticClosed { .. } => SourceTagDisposition::Suppress,
        }
    }
}

impl SourceFormattingState {
    fn tag(self) -> InlineFormattingTag {
        match self {
            Self::Open { tag, .. } | Self::SyntheticClosed { tag } => tag,
        }
    }
}

impl InlineFormattingTag {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "strong" => Some(Self::Strong),
            "b" => Some(Self::Bold),
            "em" => Some(Self::Emphasis),
            "i" => Some(Self::Italic),
            "s" => Some(Self::Strikethrough),
            "strike" => Some(Self::Strike),
            "del" => Some(Self::Delete),
            "a" => Some(Self::Link),
            "code" => Some(Self::Code),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Strong => "strong",
            Self::Bold => "b",
            Self::Emphasis => "em",
            Self::Italic => "i",
            Self::Strikethrough => "s",
            Self::Strike => "strike",
            Self::Delete => "del",
            Self::Link => "a",
            Self::Code => "code",
        }
    }
}

fn inline_content_is_empty(content: &[RichInline]) -> bool {
    content.iter().all(|inline| matches!(inline, RichInline::Text(text) if text.is_empty()))
}

fn document_semantic_markup(document: &RichDocument) -> SemanticMarkup {
    if document.blocks().iter().any(block_has_semantic_markup) {
        SemanticMarkup::Present
    } else {
        SemanticMarkup::Absent
    }
}

fn block_has_semantic_markup(block: &RichBlock) -> bool {
    match block {
        RichBlock::Paragraph(content) => inlines_have_effective_semantic(content),
        RichBlock::Heading { content, .. } => inlines_have_visible_content(content),
        RichBlock::BlockQuote(blocks) => blocks_have_visible_content(blocks),
        RichBlock::List { items, .. } => {
            items.iter().any(|blocks| blocks_have_visible_content(blocks))
        }
        RichBlock::CodeBlock { text, .. } => !text.is_empty(),
        RichBlock::Table { header, rows } => {
            cells_have_visible_content(header)
                || rows.iter().any(|row| cells_have_visible_content(row))
        }
        RichBlock::HorizontalRule => true,
    }
}

fn blocks_have_visible_content(blocks: &[RichBlock]) -> bool {
    blocks.iter().any(block_has_visible_content)
}

fn block_has_visible_content(block: &RichBlock) -> bool {
    match block {
        RichBlock::Heading { content, .. } | RichBlock::Paragraph(content) => {
            inlines_have_visible_content(content)
        }
        RichBlock::BlockQuote(blocks) => blocks_have_visible_content(blocks),
        RichBlock::List { items, .. } => {
            items.iter().any(|blocks| blocks_have_visible_content(blocks))
        }
        RichBlock::CodeBlock { text, .. } => !text.is_empty(),
        RichBlock::Table { header, rows } => {
            cells_have_visible_content(header)
                || rows.iter().any(|row| cells_have_visible_content(row))
        }
        RichBlock::HorizontalRule => true,
    }
}

fn cells_have_visible_content(cells: &[Vec<RichInline>]) -> bool {
    cells.iter().any(|content| inlines_have_visible_content(content))
}

fn inlines_have_effective_semantic(content: &[RichInline]) -> bool {
    content.iter().enumerate().any(|(index, inline)| match inline {
        RichInline::Strong(children)
        | RichInline::Emphasis(children)
        | RichInline::Strikethrough(children)
        | RichInline::Link { children, .. } => inlines_have_visible_content(children),
        RichInline::InlineCode(text) => !text.is_empty(),
        RichInline::RemoteImage { alt, .. } => !alt.trim().is_empty(),
        RichInline::LineBreak => {
            inlines_have_visible_content(&content[..index])
                && inlines_have_visible_content(&content[index + 1..])
        }
        RichInline::Text(_) => false,
    })
}

fn inlines_have_visible_content(content: &[RichInline]) -> bool {
    content.iter().any(|inline| match inline {
        RichInline::Text(text) => !text.trim().is_empty(),
        RichInline::InlineCode(text) => !text.is_empty(),
        RichInline::Strong(children)
        | RichInline::Emphasis(children)
        | RichInline::Strikethrough(children)
        | RichInline::Link { children, .. } => inlines_have_visible_content(children),
        RichInline::RemoteImage { alt, .. } => !alt.trim().is_empty(),
        RichInline::LineBreak => false,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        HtmlPasteError, MAX_HTML_NESTING_DEPTH, SemanticMarkup, normalize_malformed_fragment,
        parse_html,
    };
    use crate::paste::writer::write_markdown;
    use crate::paste::{PasteRepresentations, PreparedPaste, RichBlock, RichInline, prepare_paste};

    #[test]
    fn parses_browser_blocks_inline_styles_links_and_remote_images() {
        let conversion = parse_html(
            r#"<h2>Title</h2><p><strong>bold</strong> <a href="../a">link</a>
                <img src="img.png" alt="diagram"></p><ul><li>one</li><li>two</li></ul>"#,
            Some("https://example.com/docs/page"),
        )
        .expect("valid HTML fixture");

        assert_eq!(conversion.semantic_markup, SemanticMarkup::Present);
        assert_eq!(
            write_markdown(&conversion.document),
            "## Title\n\n**bold** [link](https://example.com/a) ![diagram](https://example.com/docs/img.png)\n\n- one\n- two"
        );
    }

    #[test]
    fn office_inline_css_maps_only_supported_semantics() {
        let conversion = parse_html(
            r#"<p><span style="font-weight:700;color:red">bold</span>
                <span style="font-style:italic;text-decoration:line-through">both</span></p>"#,
            None,
        )
        .expect("valid Office HTML fixture");
        assert_eq!(write_markdown(&conversion.document), "**bold** *~~both~~*");
    }

    #[test]
    fn highlight_only_spans_are_not_semantic_markup() {
        let conversion = parse_html(r#"<div><span style="color:#f00"># source</span></div>"#, None)
            .expect("valid highlighted source HTML");
        assert_eq!(conversion.semantic_markup, SemanticMarkup::Absent);
    }

    #[test]
    fn malformed_html_remains_parseable_and_preserves_text() {
        let conversion = parse_html("<p>one<strong>two<p>three", None)
            .expect("html5ever recovers malformed fragments");
        assert_eq!(write_markdown(&conversion.document), "one**two**\n\nthree");
    }

    #[test]
    fn converts_quote_list_code_and_table() {
        let html = r#"<blockquote><ul><li>quoted</li></ul></blockquote>
            <pre><code class="language-rust">let x = 1;</code></pre>
            <table><thead><tr><th>Name</th></tr></thead>
            <tbody><tr><td>A</td></tr></tbody></table>"#;
        let conversion = parse_html(html, None).expect("valid structural fixture");
        assert_eq!(
            write_markdown(&conversion.document),
            "> - quoted\n\n```rust\nlet x = 1;\n```\n\n| Name |\n| --- |\n| A |"
        );
    }

    #[test]
    fn rejects_embedded_images_and_active_content() {
        for source in ["data:image/png;base64,AA", "file:///tmp/a.png", "cid:image001"] {
            let html = format!(
                r#"<script>bad()</script><img src="{source}" alt="diagram" onload="bad()">"#
            );
            let conversion =
                parse_html(&html, None).expect("invalid image schemes degrade to alt text");
            assert_eq!(write_markdown(&conversion.document), "diagram");
        }
    }

    #[test]
    fn excessive_html_depth_returns_a_typed_error() {
        let html = format!(
            "{}text{}",
            "<div>".repeat(MAX_HTML_NESTING_DEPTH + 1),
            "</div>".repeat(MAX_HTML_NESTING_DEPTH + 1),
        );
        assert_eq!(parse_html(&html, None), Err(HtmlPasteError::NestingDepthExceeded));
    }

    #[test]
    fn maximum_html_depth_is_accepted() {
        let html = format!(
            "{}text{}",
            "<div>".repeat(MAX_HTML_NESTING_DEPTH),
            "</div>".repeat(MAX_HTML_NESTING_DEPTH),
        );
        let conversion = parse_html(&html, None).expect("depth at the limit remains valid");
        assert_eq!(write_markdown(&conversion.document), "text");
    }

    #[test]
    fn tag_and_supported_style_matching_is_ascii_case_insensitive() {
        let conversion = parse_html(
            r#"<P><STRONG>A</STRONG><SPAN STYLE="FONT-WEIGHT: 600; FONT-STYLE: OBLIQUE; TEXT-DECORATION: LINE-THROUGH">B</SPAN></P>"#,
            None,
        )
        .expect("mixed-case HTML and CSS");
        assert_eq!(write_markdown(&conversion.document), "**A*****~~B~~***");
    }

    #[test]
    fn decodes_entities_unicode_and_converts_breaks_headings_and_rule() {
        let conversion = parse_html(
            "<h1>A</h1><h2>B</h2><h3>C</h3><h4>D</h4><h5>E</h5><h6>F</h6><p>你好 &amp; café<br>next</p><hr>",
            None,
        )
        .expect("semantic text fixture");
        assert_eq!(
            write_markdown(&conversion.document),
            "# A\n\n## B\n\n### C\n\n#### D\n\n##### E\n\n###### F\n\n你好 & café  \nnext\n\n---"
        );
    }

    #[test]
    fn preserves_ordered_start_and_nested_list_structure() {
        let conversion = parse_html(
            "<ol start='4'><li>parent<ul><li>child</li></ul></li><li>second</li></ol>",
            None,
        )
        .expect("nested list fixture");
        assert_eq!(write_markdown(&conversion.document), "4. parent\n   - child\n5. second");
    }

    #[test]
    fn list_preserves_non_item_children_in_reading_order() {
        let conversion = parse_html(
            "<ul>before<li>one</li><custom-box>middle</custom-box><li>two</li>after</ul>",
            None,
        )
        .expect("mixed list children fixture");
        assert_eq!(
            write_markdown(&conversion.document),
            "before\n\n- one\n\nmiddle\n\n- two\n\nafter"
        );
    }

    #[test]
    fn list_ignores_invisible_non_item_children_without_splitting_items() {
        let conversion = parse_html(
            "<ul>\n <li>one</li>\n <span hidden>secret</span>\n <li>two</li>\n</ul>",
            None,
        )
        .expect("pretty-printed list fixture");
        assert_eq!(write_markdown(&conversion.document), "- one\n- two");
        assert!(matches!(
            conversion.document.blocks(),
            [RichBlock::List { items, .. }] if items.len() == 2
        ));
    }

    #[test]
    fn ordered_list_segments_saturate_start_without_panicking() {
        let conversion =
            parse_html("<ol start='18446744073709551615'><li>a</li>middle<li>b</li></ol>", None)
                .expect("maximum ordered list start fixture");
        assert_eq!(
            write_markdown(&conversion.document),
            "18446744073709551615. a\n\nmiddle\n\n18446744073709551615. b"
        );
    }

    #[test]
    fn table_without_thead_uses_first_row_and_normalizes_irregular_width() {
        let conversion = parse_html(
            "<table><tr><td>H</td></tr><tr><td>A</td><td>B</td></tr><tr><td>C</td></tr></table>",
            None,
        )
        .expect("irregular table fixture");
        assert_eq!(
            write_markdown(&conversion.document),
            "| H |  |\n| --- | --- |\n| A | B |\n| C |  |"
        );
    }

    #[test]
    fn applies_link_and_image_destination_policy() {
        let conversion = parse_html(
            r#"<p><a href="https://example.com/a">web</a> <a href="mailto:a@example.com">mail</a>
                <a href="/relative">relative</a> <a href="javascript:bad()">unsafe</a>
                <img src="https://example.com/a.png" alt="remote"><img src="/b.png" alt="relative image"></p>"#,
            None,
        )
        .expect("URL policy fixture");
        assert_eq!(
            write_markdown(&conversion.document),
            "[web](https://example.com/a) [mail](mailto:a@example.com) relative unsafe ![remote](https://example.com/a.png)relative image"
        );
        let RichBlock::Paragraph(content) = &conversion.document.blocks()[0] else {
            panic!("URL fixture should remain one paragraph");
        };
        assert!(content.iter().any(|inline| matches!(
            inline,
            RichInline::Link { destination, .. } if destination == "mailto:a@example.com"
        )));

        let relative = parse_html(
            r#"<a href="../a">link</a><img src="image.png" alt="image">"#,
            Some("https://example.com/docs/page"),
        )
        .expect("valid base URL");
        assert_eq!(
            write_markdown(&relative.document),
            "[link](https://example.com/a)![image](https://example.com/docs/image.png)"
        );

        let invalid_base = parse_html(r#"<a href="a">link</a>"#, Some("not a URL"))
            .expect("invalid base URL degrades relative destinations");
        assert_eq!(write_markdown(&invalid_base.document), "link");
    }

    #[test]
    fn skips_all_hidden_variants_and_preserves_unknown_visible_children() {
        let conversion = parse_html(
            r#"<p>shown<script>script</script><style>style</style><template>template</template>
                <span hidden>hidden</span><span aria-hidden="TRUE">aria</span>
                <span style="DISPLAY: none !important">display</span>
                <span style="visibility: HIDDEN">visibility</span><custom-box> kept</custom-box></p>"#,
            None,
        )
        .expect("hidden content fixture");
        assert_eq!(write_markdown(&conversion.document), "shown kept");
        assert_eq!(conversion.semantic_markup, SemanticMarkup::Absent);
    }

    #[test]
    fn hidden_title_does_not_leak_or_force_plain_text_fallback() {
        let html = "<title>secret</title><strong>visible</strong>";
        for plain in [None, Some("visible")] {
            let prepared = prepare_paste(PasteRepresentations {
                markdown: None,
                html: Some(html),
                rtf: None,
                plain,
                source_url: None,
            });

            assert_eq!(prepared, PreparedPaste::HtmlConverted("**visible**".into()));
        }
    }

    #[test]
    fn skips_html_non_rendered_subtrees() {
        for tag in ["head", "title", "iframe", "noembed", "noframes"] {
            let html = format!("<{tag}>secret</{tag}><strong>visible</strong>");
            let conversion = parse_html(&html, None).expect("non-rendered subtree fixture");

            assert_eq!(write_markdown(&conversion.document), "**visible**", "{tag}");
        }
    }

    #[test]
    fn raw_text_elements_with_visible_semantics_remain_visible() {
        for html in ["<textarea>visible</textarea>", "<xmp>visible</xmp>", "<plaintext>visible"] {
            let conversion = parse_html(html, None).expect("visible raw-text element fixture");

            assert_eq!(write_markdown(&conversion.document), "visible", "{html}");
        }
    }

    #[test]
    fn hidden_block_nodes_do_not_split_inline_runs() {
        let fixtures = [
            "a<div hidden>x</div>b",
            "a<div aria-hidden='true'>x</div>b",
            "a<div style='display:none'>x</div>b",
            "a<div style='visibility:hidden'>x</div>b",
            "a<script>x</script>b",
        ];
        for html in fixtures {
            let conversion = parse_html(html, None).expect("hidden block boundary fixture");
            assert_eq!(write_markdown(&conversion.document), "ab", "{html}");
        }
    }

    #[test]
    fn inline_style_cascade_respects_last_declaration_and_important() {
        let visible = parse_html("<div style='display:none;display:block'>visible</div>", None)
            .expect("later display wins");
        assert_eq!(write_markdown(&visible.document), "visible");

        let plain = parse_html("<span style='font-weight:700;font-weight:400'>plain</span>", None)
            .expect("later font weight wins");
        assert_eq!(write_markdown(&plain.document), "plain");
        assert_eq!(plain.semantic_markup, SemanticMarkup::Absent);

        let hidden =
            parse_html("<div style='display:none!important;display:block'>secret</div>", None)
                .expect("important display wins");
        assert_eq!(write_markdown(&hidden.document), "");

        let important_visible =
            parse_html("<div style='display:none;display:block!important'>visible</div>", None)
                .expect("later important display wins");
        assert_eq!(write_markdown(&important_visible.document), "visible");

        let spaced_important =
            parse_html("<div style='display:none ! important;display:block'>secret</div>", None)
                .expect("spaced important marker");
        assert_eq!(write_markdown(&spaced_important.document), "");
    }

    #[test]
    fn unknown_wrapper_preserves_block_children() {
        let conversion = parse_html("<custom-box><h2>Title</h2><p>body</p></custom-box>", None)
            .expect("unknown wrapper fixture");
        assert_eq!(write_markdown(&conversion.document), "## Title\n\nbody");
        assert_eq!(conversion.semantic_markup, SemanticMarkup::Present);
    }

    #[test]
    fn extracts_language_only_from_pre_code_language_class() {
        let conversion = parse_html(
            r#"<pre><code class="foo language-rust bar">fn main() {}</code></pre>"#,
            None,
        )
        .expect("code language fixture");
        assert_eq!(write_markdown(&conversion.document), "```rust\nfn main() {}\n```");
    }

    #[test]
    fn pre_preserves_text_around_code_child_and_uses_its_language() {
        let conversion =
            parse_html(r#"<pre>before<code class="language-rust">inside</code>after</pre>"#, None)
                .expect("mixed pre content fixture");
        assert_eq!(write_markdown(&conversion.document), "```rust\nbeforeinsideafter\n```");
    }

    #[test]
    fn table_preserves_caption_order_cell_blocks_and_ragged_rows() {
        let conversion = parse_html(
            "<table><caption>Caption</caption><tr><td><p>A</p><p>B</p></td><td>H2</td></tr><tr><td>C</td></tr></table>",
            None,
        )
        .expect("caption and cell blocks fixture");
        assert_eq!(
            write_markdown(&conversion.document),
            "Caption\n\n| A<br>B | H2 |\n| --- | --- |\n| C |  |"
        );
    }

    #[test]
    fn table_cell_preserves_nested_visible_segment_boundaries() {
        let conversion = parse_html(
            "<table><tr><td><blockquote><p>A</p><ul><li>L</li></ul><pre>B  \n  B2</pre><pre></pre><p>C</p></blockquote></td></tr></table>",
            None,
        )
        .expect("nested table cell blocks fixture");
        assert_eq!(write_markdown(&conversion.document), "| A<br>L<br>B  <br>  B2<br>C |\n| --- |");
    }

    #[test]
    fn caption_after_rows_remains_after_the_table() {
        let conversion = parse_html(
            "<table><tr><td>H</td></tr><tr><td>A</td></tr><caption>After</caption></table>",
            None,
        )
        .expect("trailing caption fixture");
        assert_eq!(write_markdown(&conversion.document), "| H |\n| --- |\n| A |\n\nAfter");
    }

    #[test]
    fn hidden_table_groups_do_not_contribute_rows() {
        let conversion = parse_html(
            "<table><thead hidden><tr><th>Secret</th></tr></thead><tbody><tr><td>Visible</td></tr></tbody></table>",
            None,
        )
        .expect("hidden table group fixture");
        assert_eq!(write_markdown(&conversion.document), "| Visible |\n| --- |");
    }

    #[test]
    fn supported_own_styles_apply_on_special_and_block_elements() {
        let conversion = parse_html(
            r#"<p style="font-weight:700">bold</p><h2 style="font-style:italic">title</h2>
                <p><a href="https://example.com" style="text-decoration:line-through">link</a>
                <code style="font-style:italic">code</code></p>"#,
            None,
        )
        .expect("own styles on semantic elements");
        assert_eq!(
            write_markdown(&conversion.document),
            "**bold**\n\n## *title*\n\n[~~link~~](https://example.com/) *`code`*"
        );
    }

    #[test]
    fn supported_own_styles_apply_to_containers_items_and_cells() {
        let conversion = parse_html(
            r#"<div style="font-weight:700">div</div>
                <blockquote style="font-style:italic"><p>quote</p></blockquote>
                <ul><li style="text-decoration:line-through">item</li></ul>
                <table><tr><th style="font-weight:bold">head</th></tr>
                <tr><td style="font-style:italic">cell</td></tr></table>"#,
            None,
        )
        .expect("container styles fixture");
        assert_eq!(
            write_markdown(&conversion.document),
            "**div**\n\n> *quote*\n\n- ~~item~~\n\n| **head** |\n| --- |\n| *cell* |"
        );
    }

    #[test]
    fn table_group_and_row_styles_apply_to_cells() {
        let conversion = parse_html(
            r#"<table><thead style="font-style:italic"><tr style="font-weight:bold">
                <th>head</th></tr></thead><tbody style="text-decoration:line-through">
                <tr><td>cell</td></tr></tbody></table>"#,
            None,
        )
        .expect("table ancestor styles fixture");
        assert_eq!(write_markdown(&conversion.document), "| ***head*** |\n| --- |\n| ~~cell~~ |");
    }

    #[test]
    fn table_ancestor_styles_use_canonical_wrapper_order() {
        let conversion = parse_html(
            r#"<table><tbody style="text-decoration:line-through"><tr style="font-weight:bold">
                <td style="font-style:italic">cell</td></tr></tbody></table>"#,
            None,
        )
        .expect("table semantic ordering fixture");
        let expected =
            vec![RichInline::Strong(vec![RichInline::Emphasis(vec![RichInline::Strikethrough(
                vec![RichInline::Text("cell".into())],
            )])])];
        let RichBlock::Table { header, .. } = &conversion.document.blocks()[0] else {
            panic!("fixture should convert to a table");
        };
        assert_eq!(header[0], expected);
    }

    #[test]
    fn table_own_style_joins_canonical_wrapper_order_without_duplicates() {
        let conversion = parse_html(
            r#"<table style="text-decoration:line-through"><tr style="font-weight:bold">
                <td style="font-weight:bold;font-style:italic">cell</td></tr></table>"#,
            None,
        )
        .expect("table own semantic ordering fixture");
        let expected =
            vec![RichInline::Strong(vec![RichInline::Emphasis(vec![RichInline::Strikethrough(
                vec![RichInline::Text("cell".into())],
            )])])];
        let RichBlock::Table { header, .. } = &conversion.document.blocks()[0] else {
            panic!("fixture should convert to a table");
        };
        assert_eq!(header[0], expected);
    }

    #[test]
    fn inherited_semantics_deduplicate_across_mixed_siblings() {
        let conversion = parse_html("<p><strong>a<strong>b</strong><em>c</em>d</strong></p>", None)
            .expect("nested semantic siblings fixture");
        let expected = vec![RichInline::Strong(vec![
            RichInline::Text("a".into()),
            RichInline::Text("b".into()),
            RichInline::Emphasis(vec![RichInline::Text("c".into())]),
            RichInline::Text("d".into()),
        ])];
        let RichBlock::Paragraph(content) = &conversion.document.blocks()[0] else {
            panic!("fixture should convert to a paragraph");
        };
        assert_eq!(*content, expected);
        assert_eq!(write_markdown(&conversion.document), "**ab*c*d**");
    }

    #[test]
    fn malformed_recovery_keeps_unrelated_legal_formatting() {
        let conversion = parse_html(
            "<p><strong>A</strong></p><p><strong>B</strong></p><p>one<strong>two<p>three",
            None,
        )
        .expect("mixed valid and malformed fixture");
        assert_eq!(write_markdown(&conversion.document), "**A**\n\n**B**\n\none**two**\n\nthree");
    }

    #[test]
    fn malformed_recovery_unwraps_all_reconstructed_children() {
        let conversion = parse_html("<p>one<strong>two<p>three<em>four", None)
            .expect("multi-child reconstructed wrapper fixture");
        assert_eq!(write_markdown(&conversion.document), "one**two**\n\nthree*four*");
    }

    #[test]
    fn malformed_scanner_ignores_fake_paragraph_tags_in_non_tag_contexts() {
        let conversion = parse_html(
            r#"<!-- <p> --><div title="<p>"></div><script>const x = '<p>';</script>
                <style>x::before { content: '<p>'; }</style><p title="<p>">one<strong>two<p>three"#,
            None,
        )
        .expect("scanner deception fixture");
        assert_eq!(write_markdown(&conversion.document), "one**two**\n\nthree");
    }

    #[test]
    fn malformed_scanner_recognizes_html5_comment_end_variants() {
        for html in [
            "<!--><p>one<strong>two<p>three",
            "<!---><p>one<strong>two<p>three",
            "<!--x--!><p>one<strong>two<p>three",
            "<!--x--><p>one<strong>two<p>three",
        ] {
            let conversion = parse_html(html, None).expect("HTML5 comment terminator fixture");
            assert_eq!(write_markdown(&conversion.document), "one**two**\n\nthree", "{html}");
        }
    }

    #[test]
    fn malformed_scanner_requires_raw_text_end_tag_boundary() {
        let conversion = parse_html(
            "<script>const x = '</scripture><p>';</script><p>one<strong>two<p>three",
            None,
        )
        .expect("raw text closing prefix fixture");
        assert_eq!(write_markdown(&conversion.document), "one**two**\n\nthree");
    }

    #[test]
    fn malformed_scanner_does_not_treat_spaced_less_than_text_as_a_tag() {
        let conversion =
            parse_html("<p>one<strong>two< p>three", None).expect("spaced less-than text fixture");
        assert_eq!(write_markdown(&conversion.document), "one**two< p>three**");

        let dotted_name = parse_html("<p>one<strong>two<p.foo>three", None)
            .expect("complete tag name boundary fixture");
        assert_eq!(write_markdown(&dotted_name.document), "one**twothree**");
    }

    #[test]
    fn formatting_start_tags_ignore_html_self_closing_flags() {
        let conversion = parse_html("<p>one<strong/>two<p>three", None)
            .expect("non-void self-closing flag fixture");
        assert_eq!(write_markdown(&conversion.document), "one**two**\n\nthree");
    }

    #[test]
    fn synthetic_closes_do_not_consume_same_named_outer_formatting() {
        let conversion =
            parse_html("<strong><p>A<strong>B<p>C</strong>D</p><p>E</p></strong>", None)
                .expect("same-name outer and inner formatting fixture");
        assert_eq!(write_markdown(&conversion.document), "**AB**\n\n**CD**\n\n**E**");

        let three_levels = parse_html(
            "<strong><p>A<strong>B<strong>C<p>D</strong>E</strong>F</p><p>G</p></strong>",
            None,
        )
        .expect("three-level same-name formatting fixture");
        assert_eq!(write_markdown(&three_levels.document), "**ABC**\n\n**DEF**\n\n**G**");
    }

    #[test]
    fn synthetic_close_suppression_is_typed_and_preserves_unclosed_outer_formatting() {
        let mixed = parse_html("<strong><p>A<em>B<p>C</em>D</p><p>E</p></strong>", None)
            .expect("different-name synthetic close fixture");
        assert_eq!(write_markdown(&mixed.document), "**A*B***\n\n**CD**\n\n**E**");

        let unclosed_inner = parse_html("<strong><p>A<strong>B<p>C</p><p>D</p></strong>E", None)
            .expect("unclosed inner formatting fixture");
        assert_eq!(write_markdown(&unclosed_inner.document), "**AB**\n\n**C**\n\n**D**\n\n**E**");
    }

    #[test]
    fn source_closes_match_the_most_recent_interleaved_formatting_state() {
        let conversion = parse_html("<p>A<strong>B<p>C</p><p><strong>D</strong>E</p>", None)
            .expect("interleaved pending and open formatting fixture");
        assert_eq!(write_markdown(&conversion.document), "A**B**\n\nC\n\n**D**E");

        let nested = parse_html(
            "<p>A<strong>B<strong>C<p>D</p><p><strong>E<strong>F</strong>G</strong>H</p>",
            None,
        )
        .expect("nested interleaved formatting fixture");
        assert_eq!(write_markdown(&nested.document), "A**BC**\n\nD\n\n**EFG**H");
    }

    #[test]
    fn raw_text_block_boundaries_stop_paragraph_formatting_leaks() {
        let xmp = parse_html("<p>one<strong>two<xmp>raw</xmp>three", None)
            .expect("xmp paragraph boundary fixture");
        assert_eq!(write_markdown(&xmp.document), "one**two**\n\nraw\n\nthree");

        let plaintext = parse_html("<p>one<strong>two<plaintext>raw</plaintext><p>three", None)
            .expect("plaintext consumes remaining source fixture");
        assert_eq!(write_markdown(&plaintext.document), "one**two**\n\nraw</plaintext><p>three");
    }

    #[test]
    fn formatting_normalizer_enforces_its_open_stack_limit() {
        let html = format!("<p>{}", "<strong>".repeat(MAX_HTML_NESTING_DEPTH + 1));
        assert_eq!(normalize_malformed_fragment(&html), Err(HtmlPasteError::NestingDepthExceeded));

        let combined_state = format!(
            "<p>{}<p>{}",
            "<strong>".repeat(MAX_HTML_NESTING_DEPTH / 2),
            "<em>".repeat(MAX_HTML_NESTING_DEPTH / 2 + 1)
        );
        assert_eq!(
            normalize_malformed_fragment(&combined_state),
            Err(HtmlPasteError::NestingDepthExceeded)
        );
    }

    #[test]
    fn malformed_scanner_stops_semantics_at_paragraph_end() {
        let conversion = parse_html(
            "<p>one<strong>two<p>three</p><div hidden><strong>hidden</strong></div>",
            None,
        )
        .expect("paragraph scope fixture");
        assert_eq!(write_markdown(&conversion.document), "one**two**\n\nthree");
    }

    #[test]
    fn malformed_recovery_visits_nested_quote_and_list_paragraphs() {
        let quote = parse_html("<blockquote><p>one<strong>two<p>three</blockquote>", None)
            .expect("nested quote malformed fixture");
        assert_eq!(write_markdown(&quote.document), "> one**two**\n> \n> three");

        let list = parse_html("<ul><li><p>one<strong>two<p>three</li></ul>", None)
            .expect("nested list malformed fixture");
        assert_eq!(write_markdown(&list.document), "- one**two**\n\n  three");
    }

    #[test]
    fn malformed_recovery_skips_source_paragraphs_without_rich_blocks() {
        let empty_prefix = parse_html("<p></p><p>one<strong>two<p>three", None)
            .expect("empty source paragraph fixture");
        assert_eq!(write_markdown(&empty_prefix.document), "one**two**\n\nthree");

        let empty_suffix = parse_html("<p>one<strong>two<p>three</p><p></p>", None)
            .expect("empty source paragraph suffix fixture");
        assert_eq!(write_markdown(&empty_suffix.document), "one**two**\n\nthree");

        let hidden_prefix = parse_html(
            "<div hidden><p><strong>hidden</strong></p></div><p>one<strong>two<p>three",
            None,
        )
        .expect("hidden source paragraph prefix fixture");
        assert_eq!(write_markdown(&hidden_prefix.document), "one**two**\n\nthree");

        let table_prefix = parse_html(
            "<table><tr><td><p>cell</p></td></tr></table><p>one<strong>two<p>three",
            None,
        )
        .expect("table cell paragraph fixture");
        assert_eq!(
            write_markdown(&table_prefix.document),
            "| cell |\n| --- |\n\none**two**\n\nthree"
        );
    }

    #[test]
    fn malformed_recovery_ignores_hidden_and_table_paragraphs_after_visible_content() {
        let hidden_suffix = parse_html(
            "<p>one<strong>two<p>three</p><div hidden><p><strong>hidden</strong></p></div>",
            None,
        )
        .expect("hidden suffix paragraph fixture");
        assert_eq!(write_markdown(&hidden_suffix.document), "one**two**\n\nthree");

        let table_suffix = parse_html(
            "<p>one<strong>two<p>three</p><table><tr><td><p><strong>cell</strong></p></td></tr></table>",
            None,
        )
        .expect("table suffix paragraph fixture");
        assert_eq!(
            write_markdown(&table_suffix.document),
            "one**two**\n\nthree\n\n| **cell** |\n| --- |"
        );
    }

    #[test]
    fn legal_outer_formatting_can_span_multiple_blocks() {
        let conversion = parse_html("<strong><p>A</p><p>B</p></strong>", None)
            .expect("legal outer formatting fixture");
        assert_eq!(write_markdown(&conversion.document), "**A**\n\n**B**");
    }

    #[test]
    fn malformed_inline_formatting_closes_at_block_boundaries() {
        let fixtures = [
            ("<p>one<strong>two<div>three</div>", "one**two**\n\nthree"),
            ("<p>one<strong>two<ul><li>three</li></ul>", "one**two**\n\n- three"),
            (
                "<p>one<strong>two<table><tr><td>three</td></tr></table>",
                "one**two**\n\n| three |\n| --- |",
            ),
        ];
        for (html, expected) in fixtures {
            let conversion = parse_html(html, None).expect("malformed block boundary fixture");
            assert_eq!(write_markdown(&conversion.document), expected, "{html}");
        }
    }

    #[test]
    fn malformed_recovery_preserves_legal_nested_formatting() {
        let conversion =
            parse_html("<p><strong>A<em>B</em></strong></p><p>one<strong>two<p>three", None)
                .expect("legal prefix plus malformed tail");
        assert_eq!(write_markdown(&conversion.document), "**A*B***\n\none**two**\n\nthree");
    }

    #[test]
    fn preformatted_break_elements_become_newlines() {
        let conversion =
            parse_html("<pre><code>a<br>b</code></pre>", None).expect("preformatted break fixture");
        assert_eq!(write_markdown(&conversion.document), "```\na\nb\n```");
    }

    #[test]
    fn semantic_markup_requires_an_effective_semantic_conversion() {
        let absent_fixtures = [
            "<p>plain</p>",
            "<unknown><span style='color:red'>plain</span></unknown>",
            "<a href='javascript:bad()'>plain</a>",
            "<img src='cid:image' alt='plain'>",
        ];
        for html in absent_fixtures {
            let conversion = parse_html(html, None).expect("non-semantic fixture");
            assert_eq!(conversion.semantic_markup, SemanticMarkup::Absent, "{html}");
        }

        for html in ["<strong>bold</strong>", "a<br>b", "<hr>", "<ul><li>x</li></ul>"] {
            let conversion = parse_html(html, None).expect("semantic fixture");
            assert_eq!(conversion.semantic_markup, SemanticMarkup::Present, "{html}");
        }
    }

    #[test]
    fn empty_semantic_nodes_do_not_mark_plain_visible_text_as_semantic() {
        let absent_fixtures = [
            "<strong></strong># source",
            "<em><strong></strong></em># source",
            "<s></s># source",
            "<a href='https://example.com'></a># source",
            "<a href='https://example.com'>   </a># source",
            "<img src='https://example.com/a.png' alt=''># source",
            "<img src='https://example.com/a.png' alt='   '># source",
            "<br># source",
        ];
        for html in absent_fixtures {
            let conversion = parse_html(html, None).expect("empty semantic fixture");
            assert_eq!(conversion.semantic_markup, SemanticMarkup::Absent, "{html}");
        }

        let effective_break = parse_html("a<br>b", None).expect("effective break fixture");
        assert_eq!(effective_break.semantic_markup, SemanticMarkup::Present);
    }
}
