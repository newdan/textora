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
        RichBlock::Heading { content, .. } => inlines_have_semantic_content(content),
        RichBlock::BlockQuote(blocks) => blocks_have_semantic_content(blocks),
        RichBlock::List { items, .. } => {
            items.iter().any(|blocks| blocks_have_semantic_content(blocks))
        }
        RichBlock::CodeBlock { text, .. } => !text.is_empty(),
        RichBlock::Table { header, rows } => {
            cells_have_semantic_content(header)
                || rows.iter().any(|row| cells_have_semantic_content(row))
        }
        RichBlock::HorizontalRule => true,
    }
}

fn blocks_have_semantic_content(blocks: &[RichBlock]) -> bool {
    blocks.iter().any(|block| block_has_visible_content(block) || block_has_semantic_markup(block))
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

fn cells_have_semantic_content(cells: &[Vec<RichInline>]) -> bool {
    cells.iter().any(|content| inlines_have_semantic_content(content))
}

fn inlines_have_semantic_content(content: &[RichInline]) -> bool {
    inlines_have_visible_content(content) || inlines_have_effective_semantic(content)
}

fn inlines_have_effective_semantic(content: &[RichInline]) -> bool {
    content.iter().enumerate().any(|(index, inline)| match inline {
        RichInline::Strong(children)
        | RichInline::Emphasis(children)
        | RichInline::Strikethrough(children)
        | RichInline::Link { children, .. } => inlines_have_semantic_content(children),
        RichInline::InlineCode(text) => !text.is_empty(),
        RichInline::RemoteImage { .. } => true,
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
#[path = "html_tests.rs"]
mod tests;
