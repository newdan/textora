#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteRepresentations<'a> {
    pub markdown: Option<&'a str>,
    pub html: Option<&'a str>,
    pub rtf: Option<&'a [u8]>,
    pub plain: Option<&'a str>,
    pub source_url: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreparedPaste {
    Markdown(String),
    HtmlConverted(String),
    RtfConverted(String),
    PlainTextFallback { text: String, reason: PasteFallbackReason },
    Empty,
}

impl PreparedPaste {
    pub fn into_text(self) -> Option<String> {
        match self {
            Self::Markdown(text)
            | Self::HtmlConverted(text)
            | Self::RtfConverted(text)
            | Self::PlainTextFallback { text, .. } => Some(text),
            Self::Empty => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteFallbackReason {
    NoSemanticHtml,
    TextMismatch,
    HtmlParseFailed,
    RtfParseFailed,
    NoRichRepresentation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RichDocument {
    blocks: Vec<RichBlock>,
}

impl RichDocument {
    pub fn new(blocks: Vec<RichBlock>) -> Self {
        Self { blocks }
    }

    pub fn blocks(&self) -> &[RichBlock] {
        &self.blocks
    }

    pub fn visible_segments(&self) -> Vec<VisibleSegment> {
        let mut collector = VisibleSegmentCollector::default();
        collector.append_blocks(&self.blocks);
        collector.finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadingLevel {
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
}

impl TryFrom<u8> for HeadingLevel {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::H1),
            2 => Ok(Self::H2),
            3 => Ok(Self::H3),
            4 => Ok(Self::H4),
            5 => Ok(Self::H5),
            6 => Ok(Self::H6),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Unordered,
    Ordered { start: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InlineSemantic {
    Strong,
    Emphasis,
    Strikethrough,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RichInline {
    Text(String),
    Strong(Vec<RichInline>),
    Emphasis(Vec<RichInline>),
    Strikethrough(Vec<RichInline>),
    InlineCode(String),
    Link { destination: String, title: Option<String>, children: Vec<RichInline> },
    RemoteImage { destination: String, title: Option<String>, alt: String },
    LineBreak,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RichBlock {
    Heading { level: HeadingLevel, content: Vec<RichInline> },
    Paragraph(Vec<RichInline>),
    BlockQuote(Vec<RichBlock>),
    List { kind: ListKind, items: Vec<Vec<RichBlock>> },
    CodeBlock { language: Option<String>, text: String },
    Table { header: Vec<Vec<RichInline>>, rows: Vec<Vec<Vec<RichInline>>> },
    HorizontalRule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibleTextMode {
    Flow,
    Preformatted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleSegment {
    pub mode: VisibleTextMode,
    pub text: String,
}

impl VisibleSegment {
    pub fn flow(text: impl Into<String>) -> Self {
        Self { mode: VisibleTextMode::Flow, text: text.into() }
    }

    pub fn preformatted(text: impl Into<String>) -> Self {
        Self { mode: VisibleTextMode::Preformatted, text: text.into() }
    }
}

#[derive(Default)]
struct VisibleSegmentCollector {
    segments: Vec<VisibleSegment>,
    flow_text: String,
}

impl VisibleSegmentCollector {
    fn append_blocks(&mut self, blocks: &[RichBlock]) {
        let mut previous_was_flow_block = false;

        for block in blocks {
            if !block.has_visible_content() {
                continue;
            }

            if previous_was_flow_block && block.is_flow_block() {
                self.flow_text.push('\n');
            }

            block.append_visible_text(self);
            previous_was_flow_block = block.is_flow_block();
        }
    }

    fn append_flow_inlines(&mut self, inlines: &[RichInline]) {
        for inline in inlines {
            inline.append_visible_text(&mut self.flow_text);
        }
    }

    fn append_table_row(&mut self, row: &[Vec<RichInline>]) {
        for (cell_index, cell) in row.iter().enumerate() {
            if cell_index > 0 {
                self.flow_text.push('\t');
            }
            self.append_flow_inlines(cell);
        }
    }

    fn append_code_block(&mut self, text: &str) {
        self.flush_flow_text();
        self.segments.push(VisibleSegment::preformatted(text));
    }

    fn flush_flow_text(&mut self) {
        if self.flow_text.is_empty() {
            return;
        }

        self.segments.push(VisibleSegment::flow(std::mem::take(&mut self.flow_text)));
    }

    fn finish(mut self) -> Vec<VisibleSegment> {
        self.flush_flow_text();
        self.segments
    }
}

impl RichBlock {
    fn has_visible_content(&self) -> bool {
        match self {
            Self::Heading { content, .. } | Self::Paragraph(content) => !content.is_empty(),
            Self::BlockQuote(blocks) => blocks.iter().any(Self::has_visible_content),
            Self::List { items, .. } => items.iter().flatten().any(Self::has_visible_content),
            Self::CodeBlock { .. } => true,
            Self::Table { header, rows } => {
                !header.is_empty() || rows.iter().any(|row| !row.is_empty())
            }
            Self::HorizontalRule => false,
        }
    }

    fn is_flow_block(&self) -> bool {
        !matches!(self, Self::CodeBlock { .. } | Self::HorizontalRule)
    }

    fn append_visible_text(&self, collector: &mut VisibleSegmentCollector) {
        match self {
            Self::Heading { content, .. } | Self::Paragraph(content) => {
                collector.append_flow_inlines(content);
            }
            Self::BlockQuote(blocks) => collector.append_blocks(blocks),
            Self::List { items, .. } => {
                for (item_index, item) in items.iter().enumerate() {
                    if item_index > 0 {
                        collector.flow_text.push('\n');
                    }
                    collector.append_blocks(item);
                }
            }
            Self::CodeBlock { text, .. } => collector.append_code_block(text),
            Self::Table { header, rows } => {
                collector.append_table_row(header);
                for row in rows {
                    collector.flow_text.push('\n');
                    collector.append_table_row(row);
                }
            }
            Self::HorizontalRule => {}
        }
    }
}

impl RichInline {
    fn append_visible_text(&self, output: &mut String) {
        match self {
            Self::Text(text) | Self::InlineCode(text) => output.push_str(text),
            Self::Strong(children)
            | Self::Emphasis(children)
            | Self::Strikethrough(children)
            | Self::Link { children, .. } => {
                for child in children {
                    child.append_visible_text(output);
                }
            }
            Self::RemoteImage { alt, .. } => output.push_str(alt),
            Self::LineBreak => output.push('\n'),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HeadingLevel, PasteFallbackReason, PreparedPaste, RichBlock, RichDocument, RichInline,
        VisibleSegment,
    };

    #[test]
    fn visible_segments_keep_code_preformatted() {
        let document = RichDocument::new(vec![
            RichBlock::Paragraph(vec![RichInline::Text("before".into())]),
            RichBlock::CodeBlock { language: Some("rust".into()), text: "let  x = 1;\n".into() },
        ]);

        assert_eq!(
            document.visible_segments(),
            vec![VisibleSegment::flow("before"), VisibleSegment::preformatted("let  x = 1;\n"),]
        );
    }

    #[test]
    fn heading_level_rejects_values_outside_one_through_six() {
        assert!(HeadingLevel::try_from(0).is_err());
        assert!(HeadingLevel::try_from(7).is_err());
    }

    #[test]
    fn visible_segments_preserve_reading_order_without_markers() {
        let document = RichDocument::new(vec![
            RichBlock::Heading {
                level: HeadingLevel::H2,
                content: vec![RichInline::Strong(vec![RichInline::Text("Title".into())])],
            },
            RichBlock::Paragraph(vec![
                RichInline::Text("Read ".into()),
                RichInline::Link {
                    destination: "https://example.com".into(),
                    title: None,
                    children: vec![RichInline::Emphasis(vec![RichInline::Text("this".into())])],
                },
                RichInline::Text(" and ".into()),
                RichInline::RemoteImage {
                    destination: "https://example.com/image.png".into(),
                    title: None,
                    alt: "diagram".into(),
                },
                RichInline::LineBreak,
                RichInline::InlineCode("next".into()),
            ]),
        ]);

        assert_eq!(
            document.visible_segments(),
            vec![VisibleSegment::flow("Title\nRead this and diagram\nnext")]
        );
    }

    #[test]
    fn prepared_paste_converts_every_textual_variant_to_text() {
        let cases = [
            (PreparedPaste::Markdown("markdown".into()), Some("markdown")),
            (PreparedPaste::HtmlConverted("html".into()), Some("html")),
            (PreparedPaste::RtfConverted("rtf".into()), Some("rtf")),
            (
                PreparedPaste::PlainTextFallback {
                    text: "plain".into(),
                    reason: PasteFallbackReason::NoRichRepresentation,
                },
                Some("plain"),
            ),
            (PreparedPaste::Empty, None),
        ];

        for (prepared, expected) in cases {
            assert_eq!(prepared.into_text().as_deref(), expected);
        }
    }

    #[test]
    fn visible_segments_flatten_nested_blocks_and_tables_in_reading_order() {
        let document = RichDocument::new(vec![
            RichBlock::BlockQuote(vec![RichBlock::Paragraph(vec![RichInline::Text(
                "quote".into(),
            )])]),
            RichBlock::List {
                kind: super::ListKind::Unordered,
                items: vec![
                    vec![RichBlock::Paragraph(vec![RichInline::Text("first".into())])],
                    vec![RichBlock::Paragraph(vec![RichInline::Text("second".into())])],
                ],
            },
            RichBlock::Table {
                header: vec![
                    vec![RichInline::Text("name".into())],
                    vec![RichInline::Text("value".into())],
                ],
                rows: vec![vec![
                    vec![RichInline::Text("answer".into())],
                    vec![RichInline::Text("42".into())],
                ]],
            },
            RichBlock::HorizontalRule,
            RichBlock::Paragraph(vec![RichInline::Text("after".into())]),
        ]);

        assert_eq!(
            document.visible_segments(),
            vec![VisibleSegment::flow("quote\nfirst\nsecond\nname\tvalue\nanswer\t42\nafter")]
        );
    }
}
