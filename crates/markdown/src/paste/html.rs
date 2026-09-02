use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Node};
use url::Url;

use super::{HeadingLevel, InlineSemantic, ListKind, RichBlock, RichDocument, RichInline};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HtmlConversion {
    pub document: RichDocument,
    pub semantic_markup: SemanticMarkup,
}

pub(crate) fn parse_html(
    html: &str,
    source_url: Option<&str>,
) -> Result<HtmlConversion, HtmlPasteError> {
    let fragment = Html::parse_fragment(html);
    let root = fragment.root_element();
    ensure_dom_depth_within_limit(*root)?;
    let base_url = source_url.and_then(|source| Url::parse(source).ok());
    let mut blocks = parse_block_children(*root, base_url.as_ref());
    if fragment.errors.iter().any(|error| error.as_ref() == "Unexpected open element") {
        remove_reconstructed_inline_formatting(&mut blocks, html);
    }
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
    let mut blocks = Vec::new();
    let mut inline_run = Vec::new();
    for child in node.children() {
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
        "ul" => vec![parse_list(element, ListKind::Unordered, base_url)],
        "ol" => vec![parse_list(element, ordered_list_kind(element), base_url)],
        "pre" => vec![parse_code_block(element)],
        "table" => parse_table(element, base_url).into_iter().collect(),
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
    let mut canonical_semantics = semantics.to_vec();
    let unwrapped_children = peel_outer_inline_semantics(children, &mut canonical_semantics);
    canonical_semantics.sort_by_key(semantic_order);
    canonical_semantics.dedup();
    canonical_semantics
        .iter()
        .rev()
        .fold(unwrapped_children, |nested, semantic| vec![wrap_inline(*semantic, nested)])
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

fn parse_list(element: ElementRef<'_>, kind: ListKind, base_url: Option<&Url>) -> RichBlock {
    let items = element
        .children()
        .filter_map(ElementRef::wrap)
        .filter(|child| child.value().name() == "li" && !element_is_hidden(*child))
        .map(|item| {
            let blocks = parse_block_children(*item, base_url);
            apply_element_semantics_to_blocks(item, blocks)
        })
        .collect();
    RichBlock::List { kind, items }
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
    let text = code.map_or_else(|| visible_text(*element), |code| visible_text(*code));
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

fn parse_table(element: ElementRef<'_>, base_url: Option<&Url>) -> Option<RichBlock> {
    let table_rows = collect_table_rows(element);
    let mut parsed_rows = table_rows
        .into_iter()
        .map(|row| parse_table_row(row, base_url))
        .filter(|row| !row.is_empty());
    let header = parsed_rows.next()?;
    Some(RichBlock::Table { header, rows: parsed_rows.collect() })
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
            let mut content = parse_inline_children(*cell, base_url);
            normalize_flow_content(&mut content);
            let semantics = canonical_table_cell_semantics(
                &row.group_semantics,
                &row_semantics,
                &inline_semantics(cell),
            );
            wrap_inline_semantics(&semantics, content)
        })
        .collect()
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
    for (property, value) in own_style_declarations(element) {
        let semantic = style_semantic(property, &value);
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
    if matches!(element.value().name(), "script" | "style" | "template") {
        return true;
    }
    if element.attr("hidden").is_some()
        || element
            .attr("aria-hidden")
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
    {
        return true;
    }
    own_style_declarations(element).any(|(property, value)| {
        matches!(
            (property, first_css_token(&value)),
            (SupportedStyleProperty::Display, "none")
                | (SupportedStyleProperty::Visibility, "hidden")
        )
    })
}

fn own_style_declarations(
    element: ElementRef<'_>,
) -> impl Iterator<Item = (SupportedStyleProperty, String)> + '_ {
    element.attr("style").into_iter().flat_map(|style| {
        style.split(';').filter_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            let property = supported_style_property(property.trim())?;
            let value = value.trim().to_ascii_lowercase();
            let value = value.strip_suffix("!important").unwrap_or(&value).trim().to_owned();
            Some((property, value))
        })
    })
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

fn remove_reconstructed_inline_formatting(blocks: &mut [RichBlock], html: &str) {
    let source_paragraphs = source_paragraph_segments(html);
    let paragraph_count =
        blocks.iter().filter(|block| matches!(block, RichBlock::Paragraph(_))).count();
    if paragraph_count != source_paragraphs.len() {
        return;
    }
    let mut source_paragraph = source_paragraphs.iter();
    for block in blocks {
        let RichBlock::Paragraph(content) = block else {
            continue;
        };
        let source = source_paragraph.next().expect("paragraph counts were checked");
        remove_unmarked_single_semantic(content, source);
    }
}

fn remove_unmarked_single_semantic(content: &mut Vec<RichInline>, source: &str) {
    if content.len() != 1 {
        return;
    }
    let Some(semantic) = inline_semantic_kind(&content[0]) else {
        return;
    };
    if source_declares_semantic(source, semantic) {
        return;
    }
    let styled_inline = content.pop().expect("one outer semantic inline was checked");
    *content = styled_inline_children(styled_inline);
}

fn inline_semantic_kind(inline: &RichInline) -> Option<InlineSemantic> {
    match inline {
        RichInline::Strong(_) => Some(InlineSemantic::Strong),
        RichInline::Emphasis(_) => Some(InlineSemantic::Emphasis),
        RichInline::Strikethrough(_) => Some(InlineSemantic::Strikethrough),
        _ => None,
    }
}

fn styled_inline_children(inline: RichInline) -> Vec<RichInline> {
    match inline {
        RichInline::Strong(children)
        | RichInline::Emphasis(children)
        | RichInline::Strikethrough(children) => children,
        inline => vec![inline],
    }
}

fn source_paragraph_segments(html: &str) -> Vec<String> {
    let lowercase_html = html.to_ascii_lowercase();
    let starts = lowercase_html
        .match_indices("<p")
        .filter(|(index, _)| is_tag_name_boundary(lowercase_html.as_bytes().get(index + 2)))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    starts
        .iter()
        .enumerate()
        .map(|(position, start)| {
            let end = starts.get(position + 1).copied().unwrap_or(lowercase_html.len());
            lowercase_html[*start..end].to_owned()
        })
        .collect()
}

fn is_tag_name_boundary(next_byte: Option<&u8>) -> bool {
    next_byte.is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'>' | b'/'))
}

fn source_declares_semantic(source: &str, semantic: InlineSemantic) -> bool {
    match semantic {
        InlineSemantic::Strong => {
            source_contains_start_tag(source, "strong")
                || source_contains_start_tag(source, "b")
                || source.contains("font-weight")
        }
        InlineSemantic::Emphasis => {
            source_contains_start_tag(source, "em")
                || source_contains_start_tag(source, "i")
                || source.contains("font-style")
        }
        InlineSemantic::Strikethrough => {
            ["s", "strike", "del"].into_iter().any(|tag| source_contains_start_tag(source, tag))
                || source.contains("text-decoration")
        }
    }
}

fn source_contains_start_tag(source: &str, tag: &str) -> bool {
    let prefix = format!("<{tag}");
    source
        .match_indices(&prefix)
        .any(|(index, _)| is_tag_name_boundary(source.as_bytes().get(index + prefix.len())))
}

fn inline_content_is_empty(content: &[RichInline]) -> bool {
    content.iter().all(|inline| matches!(inline, RichInline::Text(text) if text.is_empty()))
}

fn document_semantic_markup(document: &RichDocument) -> SemanticMarkup {
    document
        .blocks()
        .iter()
        .any(block_has_semantic_markup)
        .then_some(SemanticMarkup::Present)
        .unwrap_or(SemanticMarkup::Absent)
}

fn block_has_semantic_markup(block: &RichBlock) -> bool {
    match block {
        RichBlock::Paragraph(content) => content.iter().any(inline_has_semantic_markup),
        RichBlock::BlockQuote(blocks) => !blocks.is_empty(),
        RichBlock::Heading { .. }
        | RichBlock::List { .. }
        | RichBlock::CodeBlock { .. }
        | RichBlock::Table { .. }
        | RichBlock::HorizontalRule => true,
    }
}

fn inline_has_semantic_markup(inline: &RichInline) -> bool {
    !matches!(inline, RichInline::Text(_))
}

#[cfg(test)]
mod tests {
    use super::{HtmlPasteError, MAX_HTML_NESTING_DEPTH, SemanticMarkup, parse_html};
    use crate::paste::writer::write_markdown;
    use crate::paste::{RichBlock, RichInline};

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
            "[web](https://example.com/a) mail relative unsafe ![remote](https://example.com/a.png)relative image"
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

        for html in ["<strong>bold</strong>", "<br>", "<hr>", "<ul><li>x</li></ul>"] {
            let conversion = parse_html(html, None).expect("semantic fixture");
            assert_eq!(conversion.semantic_markup, SemanticMarkup::Present, "{html}");
        }
    }
}
