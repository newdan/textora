#[cfg(test)]
mod tests {
    use super::write_markdown;
    use crate::paste::{HeadingLevel, ListKind, RichBlock, RichDocument, RichInline};

    fn text(value: &str) -> Vec<RichInline> {
        vec![RichInline::Text(value.into())]
    }

    #[test]
    fn writes_nested_inline_styles_and_escapes_plain_markers() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
            RichInline::Text("literal * ".into()),
            RichInline::Strong(vec![RichInline::Emphasis(vec![RichInline::Text("both".into())])]),
        ])]);

        assert_eq!(write_markdown(&document), r"literal \* ***both***");
    }

    #[test]
    fn code_fence_is_longer_than_backticks_in_content() {
        let document = RichDocument::new(vec![RichBlock::CodeBlock {
            language: Some("rust".into()),
            text: "let marker = ```;\n".into(),
        }]);

        assert_eq!(write_markdown(&document), "````rust\nlet marker = ```;\n````");
    }

    #[test]
    fn writes_gfm_table_and_nested_list() {
        let document = RichDocument::new(vec![
            RichBlock::Table {
                header: vec![text("Name"), text("Value")],
                rows: vec![vec![text("A"), text("1")]],
            },
            RichBlock::List {
                kind: ListKind::Unordered,
                items: vec![vec![
                    RichBlock::Paragraph(text("parent")),
                    RichBlock::List {
                        kind: ListKind::Ordered { start: 1 },
                        items: vec![vec![RichBlock::Paragraph(text("child"))]],
                    },
                ]],
            },
        ]);

        assert_eq!(
            write_markdown(&document),
            "| Name | Value |\n| --- | --- |\n| A | 1 |\n\n- parent\n  1. child"
        );
    }

    #[test]
    fn writes_block_families_with_top_level_spacing_and_no_final_newline() {
        let document = RichDocument::new(vec![
            RichBlock::Heading { level: HeadingLevel::H2, content: text("Heading") },
            RichBlock::BlockQuote(vec![
                RichBlock::Paragraph(vec![
                    RichInline::Text("first".into()),
                    RichInline::LineBreak,
                    RichInline::Text("second".into()),
                ]),
                RichBlock::List {
                    kind: ListKind::Unordered,
                    items: vec![vec![RichBlock::Paragraph(text("item"))]],
                },
            ]),
            RichBlock::HorizontalRule,
        ]);

        assert_eq!(
            write_markdown(&document),
            "## Heading\n\n> first  \n> second\n> \n> - item\n\n---"
        );
    }

    #[test]
    fn writes_inline_code_links_and_remote_images_without_mutating_code_contents() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
            RichInline::InlineCode("tick ` marker".into()),
            RichInline::Text(" ".into()),
            RichInline::Link {
                destination: "https://example.com/a_(b)".into(),
                title: Some("a \"title\"".into()),
                children: text("link"),
            },
            RichInline::Text(" ".into()),
            RichInline::RemoteImage {
                destination: "https://example.com/image.png".into(),
                title: Some("diagram".into()),
                alt: "diagram".into(),
            },
        ])]);

        assert_eq!(
            write_markdown(&document),
            "``tick ` marker`` [link](<https://example.com/a_(b)> \"a \\\"title\\\"\") ![diagram](https://example.com/image.png \"diagram\")"
        );
    }

    #[test]
    fn normalizes_line_breaks_in_link_titles_without_changing_the_destination() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::Link {
            destination: "https://example.com".into(),
            title: Some("line\r\ntitle".into()),
            children: text("link"),
        }])]);

        assert_eq!(write_markdown(&document), "[link](https://example.com \"line title\")");
    }

    #[test]
    fn table_cells_escape_pipes_and_flatten_line_breaks() {
        let document = RichDocument::new(vec![RichBlock::Table {
            header: vec![text("A|B")],
            rows: vec![vec![vec![
                RichInline::Text("first".into()),
                RichInline::LineBreak,
                RichInline::Text("second".into()),
            ]]],
        }]);

        assert_eq!(write_markdown(&document), "| A\\|B |\n| --- |\n| first<br>second |");
    }

    #[test]
    fn preserves_ordered_list_start_numbers() {
        let document = RichDocument::new(vec![RichBlock::List {
            kind: ListKind::Ordered { start: 4 },
            items: vec![
                vec![RichBlock::Paragraph(text("first"))],
                vec![RichBlock::Paragraph(text("second"))],
            ],
        }]);

        assert_eq!(write_markdown(&document), "4. first\n5. second");
    }

    #[test]
    fn omits_empty_structures_without_creating_blank_output() {
        let document = RichDocument::new(vec![
            RichBlock::Paragraph(Vec::new()),
            RichBlock::BlockQuote(Vec::new()),
            RichBlock::List { kind: ListKind::Unordered, items: Vec::new() },
            RichBlock::Table { header: Vec::new(), rows: Vec::new() },
            RichBlock::Heading { level: HeadingLevel::H1, content: Vec::new() },
        ]);

        assert_eq!(write_markdown(&document), "");
    }

    #[test]
    fn unsafe_link_and_image_destinations_fall_back_to_visible_text() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
            RichInline::Link {
                destination: "javascript:alert(1)".into(),
                title: None,
                children: text("link"),
            },
            RichInline::Text(" ".into()),
            RichInline::RemoteImage {
                destination: "file:///private/image.png".into(),
                title: None,
                alt: "diagram".into(),
            },
        ])]);

        assert_eq!(write_markdown(&document), "link diagram");
    }

    fn representative_document() -> RichDocument {
        RichDocument::new(vec![
            RichBlock::Heading { level: HeadingLevel::H2, content: text("Heading") },
            RichBlock::List {
                kind: ListKind::Unordered,
                items: vec![vec![
                    RichBlock::Paragraph(text("parent")),
                    RichBlock::List {
                        kind: ListKind::Ordered { start: 1 },
                        items: vec![vec![RichBlock::Paragraph(text("child"))]],
                    },
                ]],
            },
            RichBlock::BlockQuote(vec![RichBlock::Paragraph(text("quoted"))]),
            RichBlock::Table { header: vec![text("Name")], rows: vec![vec![text("A")]] },
            RichBlock::CodeBlock { language: Some("rust".into()), text: "let x = 1;".into() },
            RichBlock::Paragraph(vec![RichInline::Link {
                destination: "https://example.com".into(),
                title: None,
                children: text("link"),
            }]),
        ])
    }

    #[test]
    fn representative_writer_output_reparses_without_visible_text_loss() {
        let document = representative_document();
        let markdown = write_markdown(&document);
        let parsed = crate::parser::parse_markdown(&markdown);

        assert!(!parsed.events.is_empty());
        for visible in ["Heading", "parent", "child", "quoted", "Name", "let x = 1", "link"] {
            assert!(markdown.contains(visible), "writer output lost {visible:?}: {markdown:?}");
        }
    }
}

use super::{HeadingLevel, ListKind, RichBlock, RichDocument, RichInline};

const TOP_LEVEL_BLOCK_SEPARATOR: &str = "\n\n";
const MARKDOWN_HARD_LINE_BREAK: &str = "  \n";
const FENCED_CODE_MINIMUM_DELIMITER_LENGTH: usize = 3;
const INLINE_CODE_MINIMUM_DELIMITER_LENGTH: usize = 1;
const LIST_ITEM_MARKER: &str = "- ";
const TABLE_CELL_SEPARATOR: &str = " | ";
const TABLE_BORDER: &str = "| ";
const TABLE_ROW_END: &str = " |";
const TABLE_DELIMITER_CELL: &str = "---";
const TABLE_LINE_BREAK: &str = "<br>";

#[derive(Clone, Copy)]
struct NestingContext {
    list_indentation: usize,
}

impl NestingContext {
    const TOP_LEVEL: Self = Self { list_indentation: 0 };

    fn nested_list(self, marker_width: usize) -> Self {
        Self { list_indentation: self.list_indentation + marker_width }
    }
}

#[derive(Clone, Copy)]
struct InlineContext {
    table_cell: bool,
}

impl InlineContext {
    const FLOW: Self = Self { table_cell: false };
    const TABLE_CELL: Self = Self { table_cell: true };
}

pub(crate) fn write_markdown(document: &RichDocument) -> String {
    let mut output = String::with_capacity(document.blocks().len().saturating_mul(32));
    write_blocks(document.blocks(), &mut output, NestingContext::TOP_LEVEL);
    output
}

fn write_blocks(blocks: &[RichBlock], output: &mut String, nesting: NestingContext) {
    let mut wrote_block = false;

    for block in blocks {
        let mut block_output = String::new();
        write_block(block, &mut block_output, nesting);
        if block_output.is_empty() {
            continue;
        }

        if wrote_block {
            output.push_str(TOP_LEVEL_BLOCK_SEPARATOR);
        }
        output.push_str(&block_output);
        wrote_block = true;
    }
}

fn write_block(block: &RichBlock, output: &mut String, nesting: NestingContext) {
    match block {
        RichBlock::Heading { level, content } => {
            let mut heading = String::new();
            write_inlines(content, &mut heading, InlineContext::FLOW);
            if heading.is_empty() {
                return;
            }
            output.push_str(heading_marker(*level));
            output.push(' ');
            output.push_str(&heading);
        }
        RichBlock::Paragraph(content) => write_inlines(content, output, InlineContext::FLOW),
        RichBlock::BlockQuote(blocks) => write_block_quote(blocks, output, nesting),
        RichBlock::List { kind, items } => write_list(*kind, items, output, nesting),
        RichBlock::CodeBlock { language, text } => {
            write_code_block(language.as_deref(), text, output)
        }
        RichBlock::Table { header, rows } => write_table(header, rows, output),
        RichBlock::HorizontalRule => output.push_str("---"),
    }
}

fn write_block_quote(blocks: &[RichBlock], output: &mut String, nesting: NestingContext) {
    let mut quoted = String::new();
    write_blocks(blocks, &mut quoted, nesting);
    if quoted.is_empty() {
        return;
    }

    for (line_index, line) in quoted.split('\n').enumerate() {
        if line_index > 0 {
            output.push('\n');
        }
        output.push_str("> ");
        output.push_str(line);
    }
}

fn write_list(
    kind: ListKind,
    items: &[Vec<RichBlock>],
    output: &mut String,
    nesting: NestingContext,
) {
    let mut wrote_item = false;

    for (item_index, item) in items.iter().enumerate() {
        let marker = list_item_marker(kind, item_index);
        let item_nesting = nesting.nested_list(marker.len());
        let mut item_output = String::new();
        write_list_item(item, &marker, &mut item_output, item_nesting, nesting.list_indentation);
        if item_output.is_empty() {
            continue;
        }

        if wrote_item {
            output.push('\n');
        }
        output.push_str(&item_output);
        wrote_item = true;
    }
}

fn write_list_item(
    blocks: &[RichBlock],
    marker: &str,
    output: &mut String,
    item_nesting: NestingContext,
    item_indentation: usize,
) {
    let mut wrote_block = false;

    for block in blocks {
        let mut block_output = String::new();
        write_block(block, &mut block_output, item_nesting);
        if block_output.is_empty() {
            continue;
        }

        if !wrote_block {
            output.push_str(&" ".repeat(item_indentation));
            output.push_str(marker);
            if matches!(block, RichBlock::List { .. }) {
                output.push('\n');
                output.push_str(&block_output);
            } else {
                append_with_continuation_indent(
                    output,
                    &block_output,
                    item_nesting.list_indentation,
                );
            }
            wrote_block = true;
            continue;
        }

        if matches!(block, RichBlock::List { .. }) {
            output.push('\n');
            output.push_str(&block_output);
        } else {
            output.push_str(TOP_LEVEL_BLOCK_SEPARATOR);
            output.push_str(&" ".repeat(item_nesting.list_indentation));
            append_with_continuation_indent(output, &block_output, item_nesting.list_indentation);
        }
    }
}

fn append_with_continuation_indent(output: &mut String, text: &str, indentation: usize) {
    for (line_index, line) in text.split('\n').enumerate() {
        if line_index > 0 {
            output.push('\n');
            output.push_str(&" ".repeat(indentation));
        }
        output.push_str(line);
    }
}

fn list_item_marker(kind: ListKind, item_index: usize) -> String {
    match kind {
        ListKind::Unordered => LIST_ITEM_MARKER.into(),
        ListKind::Ordered { start } => format!("{}. ", start.saturating_add(item_index as u64)),
    }
}

fn write_table(header: &[Vec<RichInline>], rows: &[Vec<Vec<RichInline>>], output: &mut String) {
    if header.is_empty() {
        return;
    }

    write_table_row(header, output);
    output.push('\n');
    write_table_delimiter_row(header.len(), output);
    for row in rows {
        output.push('\n');
        write_table_row_with_width(row, header.len(), output);
    }
}

fn write_table_row(cells: &[Vec<RichInline>], output: &mut String) {
    write_table_row_with_width(cells, cells.len(), output);
}

fn write_table_row_with_width(cells: &[Vec<RichInline>], width: usize, output: &mut String) {
    output.push_str(TABLE_BORDER);
    for cell_index in 0..width {
        if cell_index > 0 {
            output.push_str(TABLE_CELL_SEPARATOR);
        }
        if let Some(cell) = cells.get(cell_index) {
            write_inlines(cell, output, InlineContext::TABLE_CELL);
        }
    }
    output.push_str(TABLE_ROW_END);
}

fn write_table_delimiter_row(column_count: usize, output: &mut String) {
    output.push_str(TABLE_BORDER);
    for column_index in 0..column_count {
        if column_index > 0 {
            output.push_str(TABLE_CELL_SEPARATOR);
        }
        output.push_str(TABLE_DELIMITER_CELL);
    }
    output.push_str(TABLE_ROW_END);
}

fn write_code_block(language: Option<&str>, text: &str, output: &mut String) {
    let delimiter_length =
        longest_backtick_run(text).saturating_add(1).max(FENCED_CODE_MINIMUM_DELIMITER_LENGTH);
    let delimiter = "`".repeat(delimiter_length);
    output.push_str(&delimiter);
    if let Some(language) = language {
        if is_safe_fence_language(language) {
            output.push_str(language);
        }
    }
    output.push('\n');
    output.push_str(text);
    if !text.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(&delimiter);
}

fn is_safe_fence_language(language: &str) -> bool {
    !language.is_empty()
        && language.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '+' | '-')
        })
}

fn write_inlines(inlines: &[RichInline], output: &mut String, context: InlineContext) {
    for inline in inlines {
        match inline {
            RichInline::Text(text) => write_plain_text(text, output, context),
            RichInline::Strong(children) => write_styled_inlines(children, "**", output, context),
            RichInline::Emphasis(children) => write_styled_inlines(children, "*", output, context),
            RichInline::Strikethrough(children) => {
                write_styled_inlines(children, "~~", output, context)
            }
            RichInline::InlineCode(text) => write_inline_code(text, output),
            RichInline::Link { destination, title, children } => {
                write_link(destination, title.as_deref(), children, output, context)
            }
            RichInline::RemoteImage { destination, title, alt } => {
                write_remote_image(destination, title.as_deref(), alt, output, context)
            }
            RichInline::LineBreak => {
                if context.table_cell {
                    output.push_str(TABLE_LINE_BREAK);
                } else {
                    output.push_str(MARKDOWN_HARD_LINE_BREAK);
                }
            }
        }
    }
}

fn write_styled_inlines(
    children: &[RichInline],
    marker: &str,
    output: &mut String,
    context: InlineContext,
) {
    let mut child_output = String::new();
    write_inlines(children, &mut child_output, context);
    if child_output.is_empty() {
        return;
    }
    output.push_str(marker);
    output.push_str(&child_output);
    output.push_str(marker);
}

fn write_plain_text(text: &str, output: &mut String, context: InlineContext) {
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\r' => {
                if matches!(characters.peek(), Some('\n')) {
                    characters.next();
                }
                write_text_line_break(output, context);
            }
            '\n' => write_text_line_break(output, context),
            '\\' | '`' | '*' | '_' | '[' | ']' | '(' | ')' | '#' | '+' | '-' | '!' | '>' | '|' => {
                output.push('\\');
                output.push(character);
            }
            _ => output.push(character),
        }
    }
}

fn write_text_line_break(output: &mut String, context: InlineContext) {
    if context.table_cell {
        output.push_str(TABLE_LINE_BREAK);
    } else {
        output.push('\n');
    }
}

fn write_inline_code(text: &str, output: &mut String) {
    let delimiter_length =
        longest_backtick_run(text).saturating_add(1).max(INLINE_CODE_MINIMUM_DELIMITER_LENGTH);
    let delimiter = "`".repeat(delimiter_length);
    output.push_str(&delimiter);
    output.push_str(text);
    output.push_str(&delimiter);
}

fn write_link(
    destination: &str,
    title: Option<&str>,
    children: &[RichInline],
    output: &mut String,
    context: InlineContext,
) {
    if !is_safe_web_url(destination) {
        write_inlines(children, output, context);
        return;
    }

    let mut label = String::new();
    write_inlines(children, &mut label, context);
    output.push('[');
    output.push_str(&label);
    output.push_str("](");
    write_destination(destination, output);
    write_title(title, output);
    output.push(')');
}

fn write_remote_image(
    destination: &str,
    title: Option<&str>,
    alt: &str,
    output: &mut String,
    context: InlineContext,
) {
    if !is_safe_web_url(destination) {
        write_plain_text(alt, output, context);
        return;
    }

    output.push_str("![");
    write_plain_text(alt, output, context);
    output.push_str("](");
    write_destination(destination, output);
    write_title(title, output);
    output.push(')');
}

fn is_safe_web_url(destination: &str) -> bool {
    url::Url::parse(destination).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

fn write_destination(destination: &str, output: &mut String) {
    if destination
        .chars()
        .any(|character| character.is_whitespace() || matches!(character, '(' | ')'))
    {
        output.push('<');
        output.push_str(destination);
        output.push('>');
    } else {
        output.push_str(destination);
    }
}

fn write_title(title: Option<&str>, output: &mut String) {
    let Some(title) = title.filter(|title| !title.is_empty()) else {
        return;
    };

    output.push_str(" \"");
    let mut characters = title.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\\' | '\"' => {
                output.push('\\');
                output.push(character);
            }
            '\r' => {
                if matches!(characters.peek(), Some('\n')) {
                    characters.next();
                }
                output.push(' ');
            }
            '\n' => output.push(' '),
            _ => output.push(character),
        }
    }
    output.push('\"');
}

fn heading_marker(level: HeadingLevel) -> &'static str {
    match level {
        HeadingLevel::H1 => "#",
        HeadingLevel::H2 => "##",
        HeadingLevel::H3 => "###",
        HeadingLevel::H4 => "####",
        HeadingLevel::H5 => "#####",
        HeadingLevel::H6 => "######",
    }
}

fn longest_backtick_run(text: &str) -> usize {
    let mut longest_run = 0;
    let mut current_run = 0;

    for character in text.chars() {
        if character == '`' {
            current_run += 1;
            longest_run = longest_run.max(current_run);
        } else {
            current_run = 0;
        }
    }

    longest_run
}
