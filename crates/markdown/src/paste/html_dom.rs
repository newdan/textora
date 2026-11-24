use super::html::HtmlPasteError;
use super::html_entities::decode_html_entities;

const MAX_TAG_LOOKAHEAD_EMPTY: usize = 0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RawContentMode {
    RawText,
    Rcdata,
    Plaintext,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TagQuote {
    None,
    Single,
    Double,
}

pub(super) struct HtmlFragment {
    nodes: Vec<DomNode>,
}

struct DomNode {
    kind: DomNodeKind,
    children: Vec<usize>,
}

enum DomNodeKind {
    Root,
    Element(ElementData),
    Text(String),
}

struct ElementData {
    name: String,
    attrs: Vec<(String, String)>,
}

#[derive(Clone, Copy)]
pub(super) struct NodeRef<'a> {
    fragment: &'a HtmlFragment,
    id: usize,
}

#[derive(Clone, Copy)]
pub(super) struct ElementRef<'a> {
    fragment: &'a HtmlFragment,
    id: usize,
}

impl HtmlFragment {
    pub(super) fn parse(input: &str, max_depth: usize) -> Result<Self, HtmlPasteError> {
        let mut builder = DomBuilder::new(input, max_depth);
        builder.parse()?;
        Ok(Self { nodes: builder.nodes })
    }

    pub(super) fn root(&self) -> NodeRef<'_> {
        NodeRef { fragment: self, id: 0 }
    }
}

impl<'a> NodeRef<'a> {
    pub(super) fn as_text(self) -> Option<&'a str> {
        match &self.fragment.nodes[self.id].kind {
            DomNodeKind::Text(text) => Some(text.as_str()),
            DomNodeKind::Root | DomNodeKind::Element(_) => None,
        }
    }

    pub(super) fn as_element(self) -> Option<ElementRef<'a>> {
        match self.fragment.nodes[self.id].kind {
            DomNodeKind::Element(_) => Some(ElementRef { fragment: self.fragment, id: self.id }),
            DomNodeKind::Root | DomNodeKind::Text(_) => None,
        }
    }

    pub(super) fn children(self) -> impl Iterator<Item = NodeRef<'a>> {
        self.fragment.nodes[self.id]
            .children
            .iter()
            .copied()
            .map(|id| NodeRef { fragment: self.fragment, id })
    }
}

impl<'a> ElementRef<'a> {
    pub(super) fn as_node(self) -> NodeRef<'a> {
        NodeRef { fragment: self.fragment, id: self.id }
    }

    pub(super) fn name(self) -> &'a str {
        match &self.fragment.nodes[self.id].kind {
            DomNodeKind::Element(data) => data.name.as_str(),
            DomNodeKind::Root | DomNodeKind::Text(_) => {
                unreachable!("ElementRef always points to an element node")
            }
        }
    }

    pub(super) fn attr(self, name: &str) -> Option<&'a str> {
        let DomNodeKind::Element(data) = &self.fragment.nodes[self.id].kind else {
            return None;
        };
        data.attrs
            .iter()
            .find(|(attr_name, _)| attr_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub(super) fn children(self) -> impl Iterator<Item = NodeRef<'a>> {
        self.as_node().children()
    }

    pub(super) fn child_elements(self) -> impl Iterator<Item = ElementRef<'a>> {
        self.children().filter_map(NodeRef::as_element)
    }
}

struct DomBuilder<'a> {
    input: &'a str,
    bytes: &'a [u8],
    cursor: usize,
    max_depth: usize,
    nodes: Vec<DomNode>,
    open_elements: Vec<usize>,
}

impl<'a> DomBuilder<'a> {
    fn new(input: &'a str, max_depth: usize) -> Self {
        let root = DomNode { kind: DomNodeKind::Root, children: Vec::new() };
        Self {
            input,
            bytes: input.as_bytes(),
            cursor: 0,
            max_depth,
            nodes: vec![root],
            open_elements: vec![0],
        }
    }

    fn parse(&mut self) -> Result<(), HtmlPasteError> {
        while self.cursor < self.bytes.len() {
            let Some(offset) = self.bytes[self.cursor..].iter().position(|byte| *byte == b'<')
            else {
                self.append_decoded_text(&self.input[self.cursor..]);
                break;
            };
            if offset > MAX_TAG_LOOKAHEAD_EMPTY {
                let text_end = self.cursor + offset;
                self.append_decoded_text(&self.input[self.cursor..text_end]);
                self.cursor = text_end;
            }
            self.consume_markup_at_cursor()?;
        }
        Ok(())
    }

    fn consume_markup_at_cursor(&mut self) -> Result<(), HtmlPasteError> {
        match self.bytes.get(self.cursor + 1) {
            Some(b'/') => self.consume_end_tag(),
            Some(b'!') => self.consume_bang_markup(),
            Some(b'?') => self.consume_until_gt(self.cursor + 2),
            Some(byte) if byte.is_ascii_alphabetic() => return self.consume_start_tag(),
            _ => {
                self.append_raw_text("<");
                self.cursor += 1;
            }
        }
        Ok(())
    }

    fn consume_bang_markup(&mut self) {
        if self.bytes[self.cursor..].starts_with(b"<!--") {
            self.cursor = find_html_comment_end(self.bytes, self.cursor);
            return;
        }
        self.consume_until_gt(self.cursor + 2);
    }

    fn consume_until_gt(&mut self, start: usize) {
        let Some(offset) = self.bytes[start..].iter().position(|byte| *byte == b'>') else {
            self.cursor = self.bytes.len();
            return;
        };
        self.cursor = start + offset + 1;
    }

    fn consume_end_tag(&mut self) {
        let name_start = self.cursor + 2;
        let Some(&first) = self.bytes.get(name_start) else {
            self.cursor = self.bytes.len();
            return;
        };
        if first == b'>' {
            self.cursor = name_start + 1;
            return;
        }
        if !first.is_ascii_alphabetic() {
            self.consume_until_gt(name_start);
            return;
        }

        let name_end = self.scan_tag_name_end(name_start);
        let Some(tag_end) = find_tag_end(self.bytes, name_end) else {
            self.cursor = self.bytes.len();
            return;
        };
        let name = self.input[name_start..name_end].to_ascii_lowercase();
        self.cursor = tag_end + 1;
        if !is_fragment_shell_tag(&name) {
            self.close_element_in_scope(&name);
        }
    }

    fn consume_start_tag(&mut self) -> Result<(), HtmlPasteError> {
        let name_start = self.cursor + 1;
        let name_end = self.scan_tag_name_end(name_start);
        let Some((attrs, tag_end)) = self.parse_tag_attributes(name_end) else {
            self.cursor = self.bytes.len();
            return Ok(());
        };

        let name = self.input[name_start..name_end].to_ascii_lowercase();
        self.cursor = tag_end + 1;
        if is_fragment_shell_tag(&name) {
            return Ok(());
        }

        self.apply_implicit_closes(&name);
        let node_id = self.push_element_node(name.clone(), attrs)?;
        if is_void_element(&name) {
            return Ok(());
        }

        self.open_elements.push(node_id);
        if name == "pre" {
            self.strip_single_leading_newline();
            return Ok(());
        }
        if let Some(raw_mode) = raw_content_mode(&name) {
            self.consume_raw_or_rcdata_element(&name, raw_mode);
        }
        Ok(())
    }

    fn scan_tag_name_end(&self, start: usize) -> usize {
        let mut pos = start;
        while let Some(&byte) = self.bytes.get(pos) {
            if byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>') {
                break;
            }
            pos += 1;
        }
        pos
    }

    fn parse_tag_attributes(&self, mut pos: usize) -> Option<(Vec<(String, String)>, usize)> {
        let mut attrs = Vec::new();
        loop {
            pos = skip_ascii_whitespace_and_slashes(self.bytes, pos);
            let &byte = self.bytes.get(pos)?;
            if byte == b'>' {
                return Some((attrs, pos));
            }
            let (name, value, next_pos) = self.parse_single_attribute(pos)?;
            if !name.is_empty() && !attrs.iter().any(|(existing, _)| existing == &name) {
                attrs.push((name, value));
            }
            pos = next_pos;
        }
    }

    fn parse_single_attribute(&self, start: usize) -> Option<(String, String, usize)> {
        let mut pos = start;
        while let Some(&byte) = self.bytes.get(pos) {
            if byte.is_ascii_whitespace() || matches!(byte, b'=' | b'/' | b'>') {
                break;
            }
            pos += 1;
        }
        self.bytes.get(pos)?;
        let name = self.input[start..pos].to_ascii_lowercase();
        pos = skip_ascii_whitespace(self.bytes, pos);
        if self.bytes.get(pos) != Some(&b'=') {
            return Some((name, String::new(), pos));
        }

        pos = skip_ascii_whitespace(self.bytes, pos + 1);
        let (value, after_value) = self.parse_attribute_value(pos)?;
        Some((name, value, after_value))
    }

    fn parse_attribute_value(&self, start: usize) -> Option<(String, usize)> {
        let &first = self.bytes.get(start)?;
        if first == b'"' || first == b'\'' {
            let value_start = start + 1;
            let offset = self.bytes[value_start..].iter().position(|byte| *byte == first)?;
            let value_end = value_start + offset;
            let decoded = decode_html_entities(&self.input[value_start..value_end], true);
            return Some((decoded, value_end + 1));
        }
        if first == b'>' {
            return Some((String::new(), start));
        }

        let mut pos = start;
        while let Some(&byte) = self.bytes.get(pos) {
            if byte.is_ascii_whitespace() || byte == b'>' {
                break;
            }
            pos += 1;
        }
        self.bytes.get(pos)?;
        let decoded = decode_html_entities(&self.input[start..pos], true);
        Some((decoded, pos))
    }

    fn push_element_node(
        &mut self,
        name: String,
        attrs: Vec<(String, String)>,
    ) -> Result<usize, HtmlPasteError> {
        if self.open_elements.len() > self.max_depth {
            return Err(HtmlPasteError::NestingDepthExceeded);
        }
        let node_id = self.nodes.len();
        self.nodes.push(DomNode {
            kind: DomNodeKind::Element(ElementData { name, attrs }),
            children: Vec::new(),
        });
        let parent_id = *self.open_elements.last().expect("open_elements always keeps root");
        self.nodes[parent_id].children.push(node_id);
        Ok(node_id)
    }

    fn append_decoded_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let decoded = decode_html_entities(text, false);
        self.append_raw_text(&decoded);
    }

    fn append_raw_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let parent_id = *self.open_elements.last().expect("open_elements always keeps root");
        if let Some(&last_child_id) = self.nodes[parent_id].children.last()
            && let DomNodeKind::Text(existing) = &mut self.nodes[last_child_id].kind
        {
            existing.push_str(text);
            return;
        }
        let node_id = self.nodes.len();
        self.nodes.push(DomNode { kind: DomNodeKind::Text(text.to_owned()), children: Vec::new() });
        self.nodes[parent_id].children.push(node_id);
    }

    fn strip_single_leading_newline(&mut self) {
        if self.bytes[self.cursor..].starts_with(b"\r\n") {
            self.cursor += 2;
        } else if self.bytes.get(self.cursor) == Some(&b'\n') {
            self.cursor += 1;
        }
    }

    fn consume_raw_or_rcdata_element(&mut self, name: &str, mode: RawContentMode) {
        if mode == RawContentMode::Plaintext {
            self.append_raw_text(&self.input[self.cursor..]);
            self.cursor = self.bytes.len();
            return;
        }
        if name == "textarea" {
            self.strip_single_leading_newline();
        }
        let content_start = self.cursor;
        let Some((close_start, close_end)) = find_raw_closing_tag(self.bytes, content_start, name)
        else {
            self.append_raw_or_rcdata(&self.input[content_start..], mode);
            self.cursor = self.bytes.len();
            return;
        };
        self.append_raw_or_rcdata(&self.input[content_start..close_start], mode);
        self.cursor = close_end + 1;
        if self.open_elements.len() > 1 {
            self.open_elements.pop();
        }
    }

    fn append_raw_or_rcdata(&mut self, text: &str, mode: RawContentMode) {
        match mode {
            RawContentMode::Rcdata => self.append_decoded_text(text),
            RawContentMode::RawText | RawContentMode::Plaintext => self.append_raw_text(text),
        }
    }

    fn apply_implicit_closes(&mut self, tag: &str) {
        if closes_open_paragraph(tag) {
            self.pop_to_if_in_scope("p", is_button_scope_barrier);
        }
        if is_heading_tag(tag) && self.top_element_name().is_some_and(is_heading_tag) {
            self.open_elements.pop();
        }
        if tag == "li" {
            self.pop_to_if_in_scope("li", is_list_scope_barrier);
        } else if matches!(tag, "dd" | "dt") {
            self.pop_to_any_in_scope(&["dd", "dt"], is_dl_scope_barrier);
        } else if is_table_child_tag(tag) && self.has_table_in_scope() {
            self.clear_table_context(tag);
        }
    }

    fn clear_table_context(&mut self, tag: &str) {
        let stop_tags: &[&str] = match tag {
            "td" | "th" => &["tr", "thead", "tbody", "tfoot", "table"],
            "tr" => &["thead", "tbody", "tfoot", "table"],
            _ => &["table"],
        };
        while self.open_elements.len() > 1 {
            let Some(top_name) = self.top_element_name() else {
                break;
            };
            if stop_tags.contains(&top_name) || top_name == "template" {
                break;
            }
            self.open_elements.pop();
        }
    }

    fn close_element_in_scope(&mut self, tag: &str) {
        match tag {
            "p" => self.pop_to_if_in_scope("p", is_button_scope_barrier),
            "li" => self.pop_to_if_in_scope("li", is_list_scope_barrier),
            "dd" | "dt" => self.pop_to_if_in_scope(tag, is_dl_scope_barrier),
            "table" => self.pop_to_if_in_scope("table", is_template_barrier),
            "td" | "th" => self.pop_to_if_in_scope(tag, is_table_cell_scope_barrier),
            "tr" | "thead" | "tbody" | "tfoot" | "caption" | "colgroup" => {
                self.pop_to_if_in_scope(tag, is_table_section_scope_barrier);
            }
            _ => self.pop_to_if_in_scope(tag, is_general_scope_barrier),
        }
    }

    fn pop_to_if_in_scope(&mut self, target: &str, is_barrier: fn(&str) -> bool) {
        self.pop_to_any_in_scope(&[target], is_barrier);
    }

    fn pop_to_any_in_scope(&mut self, targets: &[&str], is_barrier: fn(&str) -> bool) {
        for stack_index in (1..self.open_elements.len()).rev() {
            let name = self.element_name(self.open_elements[stack_index]);
            if targets.contains(&name) {
                self.open_elements.truncate(stack_index);
                return;
            }
            if is_barrier(name) {
                return;
            }
        }
    }

    fn has_table_in_scope(&self) -> bool {
        for stack_index in (1..self.open_elements.len()).rev() {
            let name = self.element_name(self.open_elements[stack_index]);
            if name == "table" {
                return true;
            }
            if name == "template" {
                return false;
            }
        }
        false
    }

    fn top_element_name(&self) -> Option<&str> {
        let id = *self.open_elements.last()?;
        (id != 0).then(|| self.element_name(id))
    }

    fn element_name(&self, id: usize) -> &str {
        match &self.nodes[id].kind {
            DomNodeKind::Element(data) => data.name.as_str(),
            DomNodeKind::Root | DomNodeKind::Text(_) => "",
        }
    }
}

fn find_html_comment_end(bytes: &[u8], tag_start: usize) -> usize {
    const STANDARD_COMMENT_END: &[u8] = b"-->";
    const BANG_COMMENT_END: &[u8] = b"--!>";
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

fn find_tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut quote = TagQuote::None;
    for (offset, &byte) in bytes[start..].iter().enumerate() {
        quote = match (quote, byte) {
            (TagQuote::None, b'\'') => TagQuote::Single,
            (TagQuote::None, b'"') => TagQuote::Double,
            (TagQuote::Single, b'\'') | (TagQuote::Double, b'"') => TagQuote::None,
            (current, _) => current,
        };
        if quote == TagQuote::None && byte == b'>' {
            return Some(start + offset);
        }
    }
    None
}

fn find_raw_closing_tag(bytes: &[u8], start: usize, tag_name: &str) -> Option<(usize, usize)> {
    let name_bytes = tag_name.as_bytes();
    let prefix_len = 2 + name_bytes.len();
    let mut pos = start;
    while pos + prefix_len <= bytes.len() {
        if bytes[pos] == b'<'
            && bytes[pos + 1] == b'/'
            && bytes[pos + 2..pos + prefix_len].eq_ignore_ascii_case(name_bytes)
        {
            let boundary = bytes.get(pos + prefix_len);
            if boundary
                .is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
                && let Some(tag_end) = find_tag_end(bytes, pos + prefix_len)
            {
                return Some((pos, tag_end));
            }
        }
        pos += 1;
    }
    None
}

fn skip_ascii_whitespace(bytes: &[u8], mut pos: usize) -> usize {
    while bytes.get(pos).is_some_and(u8::is_ascii_whitespace) {
        pos += 1;
    }
    pos
}

fn skip_ascii_whitespace_and_slashes(bytes: &[u8], mut pos: usize) -> usize {
    while bytes.get(pos).is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'/') {
        pos += 1;
    }
    pos
}

fn is_fragment_shell_tag(name: &str) -> bool {
    matches!(name, "html" | "body")
}

fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn raw_content_mode(name: &str) -> Option<RawContentMode> {
    match name {
        "script" | "style" | "xmp" | "iframe" | "noembed" | "noframes" => {
            Some(RawContentMode::RawText)
        }
        "textarea" | "title" => Some(RawContentMode::Rcdata),
        "plaintext" => Some(RawContentMode::Plaintext),
        _ => None,
    }
}

fn closes_open_paragraph(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "details"
            | "dialog"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
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
            | "hgroup"
            | "hr"
            | "li"
            | "main"
            | "menu"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "plaintext"
            | "search"
            | "section"
            | "table"
            | "ul"
            | "xmp"
    )
}

fn is_heading_tag(name: &str) -> bool {
    matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

fn is_table_child_tag(name: &str) -> bool {
    matches!(name, "caption" | "thead" | "tbody" | "tfoot" | "colgroup" | "tr" | "td" | "th")
}

fn is_button_scope_barrier(name: &str) -> bool {
    matches!(
        name,
        "table" | "td" | "th" | "caption" | "template" | "button" | "object" | "marquee" | "applet"
    )
}

fn is_list_scope_barrier(name: &str) -> bool {
    matches!(
        name,
        "ul" | "ol" | "menu" | "table" | "td" | "th" | "caption" | "blockquote" | "template"
    )
}

fn is_dl_scope_barrier(name: &str) -> bool {
    matches!(name, "dl" | "table" | "td" | "th" | "caption" | "template")
}

fn is_template_barrier(name: &str) -> bool {
    name == "template"
}

fn is_table_cell_scope_barrier(name: &str) -> bool {
    matches!(name, "tr" | "table" | "template")
}

fn is_table_section_scope_barrier(name: &str) -> bool {
    matches!(name, "table" | "template")
}

fn is_general_scope_barrier(name: &str) -> bool {
    matches!(name, "table" | "td" | "th" | "caption" | "template")
}

#[cfg(test)]
mod tests {
    use super::HtmlFragment;
    use crate::paste::html::HtmlPasteError;

    #[test]
    fn parses_quoted_unquoted_boolean_and_duplicate_attributes() {
        let fragment = HtmlFragment::parse(
            r#"<a HREF="https://a.test/?x=1&amp;copy=2&copy=3" title='single' data-k=unquoted hidden href="https://ignored.test">ok</a>"#,
            32,
        )
        .expect("attributes should parse");
        let link = fragment.root().children().next().and_then(|n| n.as_element()).expect("<a>");
        assert_eq!(link.name(), "a");
        assert_eq!(link.attr("href"), Some("https://a.test/?x=1&copy=2&copy=3"));
        assert_eq!(link.attr("title"), Some("single"));
        assert_eq!(link.attr("data-k"), Some("unquoted"));
        assert_eq!(link.attr("hidden"), Some(""));
    }

    #[test]
    fn unwraps_fragment_shell_tags_and_ignores_non_void_self_closing() {
        let fragment =
            HtmlFragment::parse("<html><body><p>a<strong/>b<br/>c</p></body></html>", 32)
                .expect("fragment shell should unwrap");
        let elements: Vec<_> = fragment.root().children().filter_map(|n| n.as_element()).collect();
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].name(), "p");
        let children: Vec<_> = elements[0].children().collect();
        assert_eq!(children[0].as_text(), Some("a"));
        let strong = children[1].as_element().expect("strong must stay open");
        assert_eq!(strong.name(), "strong");
        let strong_children: Vec<_> = strong.children().collect();
        assert_eq!(strong_children[0].as_text(), Some("b"));
        assert_eq!(strong_children[1].as_element().expect("br").name(), "br");
        assert_eq!(strong_children[2].as_text(), Some("c"));
    }

    #[test]
    fn implicitly_closes_paragraphs_list_items_and_tables() {
        let fragment = HtmlFragment::parse(
            "<p>one<p>two<ul><li>a<li>b</ul><table><tr><td>1<td>2<tr><td>3</table>",
            32,
        )
        .expect("implicit closes should succeed");
        let top: Vec<_> =
            fragment.root().children().filter_map(|n| n.as_element()).map(|e| e.name()).collect();
        assert_eq!(top, ["p", "p", "ul", "table"]);

        let ul = fragment.root().children().filter_map(|n| n.as_element()).nth(2).expect("ul");
        assert_eq!(ul.child_elements().count(), 2);

        let table =
            fragment.root().children().filter_map(|n| n.as_element()).nth(3).expect("table");
        let rows: Vec<_> = table.child_elements().collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].child_elements().count(), 2);
        assert_eq!(rows[1].child_elements().count(), 1);
    }

    #[test]
    fn handles_comments_raw_text_rcdata_and_pre_leading_newline() {
        let fragment = HtmlFragment::parse(
            "<!-->a<!---><script>const x='</div>&amp;';</script><textarea>\nhello &amp; world</textarea><pre>\ncode\n</pre>",
            32,
        )
        .expect("special elements should parse");
        let children: Vec<_> = fragment.root().children().collect();
        assert_eq!(children[0].as_text(), Some("a"));
        let script = children[1].as_element().expect("script");
        assert_eq!(
            script.children().next().and_then(|n| n.as_text()),
            Some("const x='</div>&amp;';")
        );
        let textarea = children[2].as_element().expect("textarea");
        assert_eq!(textarea.children().next().and_then(|n| n.as_text()), Some("hello & world"));
        let pre = children[3].as_element().expect("pre");
        assert_eq!(pre.children().next().and_then(|n| n.as_text()), Some("code\n"));
    }

    #[test]
    fn enforces_exact_depth_limit_without_false_positive_on_implicit_close() {
        let at_limit = format!("{}ok{}", "<div>".repeat(4), "</div>".repeat(4));
        assert!(HtmlFragment::parse(&at_limit, 4).is_ok());

        let over_limit = format!("{}ok{}", "<div>".repeat(5), "</div>".repeat(5));
        assert_eq!(
            HtmlFragment::parse(&over_limit, 4).err(),
            Some(HtmlPasteError::NestingDepthExceeded)
        );

        let unclosed_paragraphs = "<p>item".repeat(300);
        assert!(HtmlFragment::parse(&unclosed_paragraphs, 4).is_ok());
    }
}
