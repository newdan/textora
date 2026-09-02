use super::{
    PasteFallbackReason, PasteRepresentations, PreparedPaste, RichDocument, VisibleSegment,
    VisibleTextMode,
    html::{SemanticMarkup, parse_html},
    rtf::parse_rtf,
    writer::write_markdown,
};

enum HtmlSelection {
    Prepared(PreparedPaste),
    TryRtf(PasteFallbackReason),
}

#[derive(Clone, Copy)]
enum RichRepresentation {
    Html,
    Rtf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum VisibleChunk {
    Flow(String),
    Preformatted(String),
}

#[derive(Clone, Copy)]
enum PatternToken {
    Literal(char),
    WhitespaceOne,
    WhitespaceStar,
}

pub fn prepare_paste(input: PasteRepresentations<'_>) -> PreparedPaste {
    if let Some(markdown) = non_empty_text(input.markdown) {
        return PreparedPaste::Markdown(markdown.to_owned());
    }

    let plain = non_empty_text(input.plain);
    match select_html(input.html, input.source_url, plain) {
        HtmlSelection::Prepared(prepared) => prepared,
        HtmlSelection::TryRtf(fallback_reason) => select_rtf(input.rtf, plain, fallback_reason),
    }
}

fn select_html(html: Option<&str>, source_url: Option<&str>, plain: Option<&str>) -> HtmlSelection {
    let Some(html) = non_empty_text(html) else {
        return HtmlSelection::TryRtf(PasteFallbackReason::NoRichRepresentation);
    };
    let Ok(conversion) = parse_html(html, source_url) else {
        return HtmlSelection::TryRtf(PasteFallbackReason::HtmlParseFailed);
    };

    match conversion.semantic_markup {
        SemanticMarkup::Present => HtmlSelection::Prepared(select_converted_document(
            &conversion.document,
            plain,
            RichRepresentation::Html,
        )),
        SemanticMarkup::Absent => match plain {
            Some(text) => {
                HtmlSelection::Prepared(plain_fallback(text, PasteFallbackReason::NoSemanticHtml))
            }
            None => HtmlSelection::TryRtf(PasteFallbackReason::NoRichRepresentation),
        },
    }
}

fn select_rtf(
    rtf: Option<&[u8]>,
    plain: Option<&str>,
    fallback_reason: PasteFallbackReason,
) -> PreparedPaste {
    let Some(rtf) = rtf.filter(|bytes| !bytes.is_empty()) else {
        return plain_or_empty(plain, fallback_reason);
    };
    let Ok(document) = parse_rtf(rtf) else {
        return plain_or_empty(plain, PasteFallbackReason::RtfParseFailed);
    };

    select_converted_document(&document, plain, RichRepresentation::Rtf)
}

fn select_converted_document(
    document: &RichDocument,
    plain: Option<&str>,
    representation: RichRepresentation,
) -> PreparedPaste {
    if let Some(text) = plain.filter(|text| !equivalent_visible_text(document, text)) {
        return plain_fallback(text, PasteFallbackReason::TextMismatch);
    }

    let markdown = write_markdown(document);
    if markdown.trim().is_empty() {
        return PreparedPaste::Empty;
    }
    match representation {
        RichRepresentation::Html => PreparedPaste::HtmlConverted(markdown),
        RichRepresentation::Rtf => PreparedPaste::RtfConverted(markdown),
    }
}

fn plain_or_empty(plain: Option<&str>, reason: PasteFallbackReason) -> PreparedPaste {
    plain.map(|text| plain_fallback(text, reason)).unwrap_or(PreparedPaste::Empty)
}

fn plain_fallback(text: &str, reason: PasteFallbackReason) -> PreparedPaste {
    PreparedPaste::PlainTextFallback { text: text.to_owned(), reason }
}

fn non_empty_text(text: Option<&str>) -> Option<&str> {
    text.filter(|value| !value.is_empty())
}

fn equivalent_visible_text(document: &RichDocument, plain: &str) -> bool {
    preformatted_segments_appear_in_order(document, plain)
}

fn segments_align_plain(segments: &[VisibleSegment], plain: &str) -> bool {
    let visible_text =
        segments.iter().map(|segment| segment.text.as_str()).collect::<Vec<_>>().join("\n");
    if normalize_flow_text(&visible_text) != normalize_flow_text(plain) {
        return false;
    }

    let pattern = compile_visible_pattern(segments);
    let normalized_plain = normalize_line_endings(plain);
    let plain_without_bom = normalized_plain.strip_prefix('\u{feff}').unwrap_or(&normalized_plain);
    pattern_matches_plain(&pattern, plain_without_bom)
}

fn normalize_flow_text(text: &str) -> String {
    normalize_flow_fragment(text, true)
}

fn normalize_flow_fragment(text: &str, strip_leading_bom: bool) -> String {
    let without_bom =
        if strip_leading_bom { text.strip_prefix('\u{feff}').unwrap_or(text) } else { text };
    let normalized_endings = normalize_line_endings(without_bom);
    normalized_endings
        .chars()
        .map(|character| if character == '\u{a0}' { ' ' } else { character })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn preformatted_segments_appear_in_order(document: &RichDocument, plain: &str) -> bool {
    segments_align_plain(&document.visible_segments(), plain)
}

fn merge_visible_segments(segments: &[VisibleSegment]) -> Vec<VisibleChunk> {
    let mut chunks = Vec::with_capacity(segments.len().saturating_add(2));
    for (index, segment) in segments.iter().enumerate() {
        match segment.mode {
            VisibleTextMode::Flow => append_flow_chunk(&mut chunks, &segment.text),
            VisibleTextMode::Preformatted => {
                append_preformatted_chunk(&mut chunks, &segment.text, index == 0);
            }
        }
    }
    if !matches!(chunks.last(), Some(VisibleChunk::Flow(_))) {
        chunks.push(VisibleChunk::Flow(String::new()));
    }
    for (index, chunk) in chunks.iter_mut().enumerate() {
        if let VisibleChunk::Flow(flow) = chunk {
            *flow = normalize_flow_fragment(flow, index == 0);
        }
    }
    chunks
}

fn append_flow_chunk(chunks: &mut Vec<VisibleChunk>, text: &str) {
    match chunks.last_mut() {
        Some(VisibleChunk::Flow(flow)) => {
            flow.push('\n');
            flow.push_str(text);
        }
        _ => chunks.push(VisibleChunk::Flow(text.to_owned())),
    }
}

fn append_preformatted_chunk(chunks: &mut Vec<VisibleChunk>, text: &str, strip_bom: bool) {
    if !matches!(chunks.last(), Some(VisibleChunk::Flow(_))) {
        chunks.push(VisibleChunk::Flow(String::new()));
    }
    let text = if strip_bom { text.strip_prefix('\u{feff}').unwrap_or(text) } else { text };
    chunks.push(VisibleChunk::Preformatted(normalize_line_endings(text)));
}

fn compile_visible_pattern(segments: &[VisibleSegment]) -> Vec<PatternToken> {
    let chunks = merge_visible_segments(segments);
    let mut pattern = Vec::new();
    for chunk in chunks {
        match chunk {
            VisibleChunk::Flow(flow) => append_flow_pattern(&mut pattern, &flow),
            VisibleChunk::Preformatted(text) => {
                pattern.extend(text.chars().map(PatternToken::Literal));
            }
        }
    }
    pattern
}

fn append_flow_pattern(pattern: &mut Vec<PatternToken>, flow: &str) {
    pattern.push(PatternToken::WhitespaceStar);
    for character in flow.chars() {
        if character == ' ' {
            pattern.push(PatternToken::WhitespaceOne);
            pattern.push(PatternToken::WhitespaceStar);
        } else {
            pattern.push(PatternToken::Literal(character));
        }
    }
    pattern.push(PatternToken::WhitespaceStar);
}

fn pattern_matches_plain(pattern: &[PatternToken], plain: &str) -> bool {
    let mut active = vec![false; pattern.len() + 1];
    let mut next = vec![false; pattern.len() + 1];
    active[0] = true;
    apply_epsilon_closure(pattern, &mut active);
    for character in plain.chars() {
        next.fill(false);
        advance_pattern(pattern, &active, character, &mut next);
        apply_epsilon_closure(pattern, &mut next);
        if !next.iter().any(|state| *state) {
            return false;
        }
        std::mem::swap(&mut active, &mut next);
    }
    active[pattern.len()]
}

fn advance_pattern(pattern: &[PatternToken], active: &[bool], character: char, next: &mut [bool]) {
    for (index, token) in pattern.iter().enumerate() {
        if !active[index] {
            continue;
        }
        match token {
            PatternToken::Literal(expected) if *expected == character => next[index + 1] = true,
            PatternToken::WhitespaceOne if is_flow_whitespace(character) => {
                next[index + 1] = true;
            }
            PatternToken::WhitespaceStar if is_flow_whitespace(character) => next[index] = true,
            _ => {}
        }
    }
}

fn apply_epsilon_closure(pattern: &[PatternToken], active: &mut [bool]) {
    for (index, token) in pattern.iter().enumerate() {
        if active[index] && matches!(token, PatternToken::WhitespaceStar) {
            active[index + 1] = true;
        }
    }
}

fn is_flow_whitespace(character: char) -> bool {
    character == '\u{a0}' || character.is_whitespace()
}

#[cfg(test)]
mod tests {
    use super::{
        equivalent_visible_text, preformatted_segments_appear_in_order, prepare_paste,
        segments_align_plain,
    };
    use crate::paste::{
        PasteFallbackReason, PasteRepresentations, PreparedPaste, RichBlock, RichDocument,
        RichInline, VisibleSegment,
    };

    #[test]
    fn explicit_markdown_wins_without_visible_text_comparison() {
        let prepared = prepare_paste(PasteRepresentations {
            markdown: Some("**source**"),
            html: Some("<strong>source</strong>"),
            rtf: None,
            plain: Some("source"),
            source_url: None,
        });
        assert_eq!(prepared, PreparedPaste::Markdown("**source**".into()));
    }

    #[test]
    fn semantic_html_wins_when_visible_text_matches() {
        let prepared = prepare_paste(PasteRepresentations {
            markdown: None,
            html: Some("<p><strong>same</strong></p>"),
            rtf: None,
            plain: Some("same"),
            source_url: None,
        });
        assert_eq!(prepared, PreparedPaste::HtmlConverted("**same**".into()));
    }

    #[test]
    fn highlighted_markdown_source_uses_plain_text() {
        let prepared = prepare_paste(PasteRepresentations {
            markdown: None,
            html: Some("<div><span style='color:red'># source</span></div>"),
            rtf: Some(br"{\rtf1\cf1 # source}"),
            plain: Some("# source"),
            source_url: None,
        });
        assert_eq!(
            prepared,
            PreparedPaste::PlainTextFallback {
                text: "# source".into(),
                reason: PasteFallbackReason::NoSemanticHtml,
            }
        );
    }

    #[test]
    fn mismatch_rtf_and_empty_cases_follow_the_priority_contract() {
        assert!(matches!(
            prepare_paste(PasteRepresentations {
                markdown: None,
                html: Some("<p><strong>different</strong></p>"),
                rtf: None,
                plain: Some("plain"),
                source_url: None,
            }),
            PreparedPaste::PlainTextFallback { reason: PasteFallbackReason::TextMismatch, .. }
        ));

        assert!(matches!(
            prepare_paste(PasteRepresentations {
                markdown: None,
                html: None,
                rtf: Some(br"{\rtf1\b rich\b0}"),
                plain: Some("rich"),
                source_url: None,
            }),
            PreparedPaste::RtfConverted(ref text) if text == "**rich**"
        ));

        assert_eq!(
            prepare_paste(PasteRepresentations {
                markdown: None,
                html: None,
                rtf: None,
                plain: None,
                source_url: None,
            }),
            PreparedPaste::Empty
        );
    }

    #[test]
    fn unsafe_html_depth_falls_through_to_rtf() {
        let html = format!(
            "{}rich{}",
            "<div>".repeat(crate::paste::html::MAX_HTML_NESTING_DEPTH + 1),
            "</div>".repeat(crate::paste::html::MAX_HTML_NESTING_DEPTH + 1),
        );
        let prepared = prepare_paste(PasteRepresentations {
            markdown: None,
            html: Some(&html),
            rtf: Some(br"{\rtf1\b rich\b0}"),
            plain: Some("rich"),
            source_url: None,
        });
        assert_eq!(prepared, PreparedPaste::RtfConverted("**rich**".into()));
    }

    #[test]
    fn rich_representation_is_used_when_plain_is_absent() {
        let prepared = prepare_paste(PasteRepresentations {
            markdown: None,
            html: Some("<p><strong>rich</strong></p>"),
            rtf: None,
            plain: None,
            source_url: None,
        });
        assert_eq!(prepared, PreparedPaste::HtmlConverted("**rich**".into()));
    }

    #[test]
    fn plain_text_keeps_line_breaks_and_unicode_bytes() {
        let plain = "a\nb\n\nc 你好 🌍 e\u{301}";
        let prepared = prepare_paste(PasteRepresentations {
            markdown: None,
            html: None,
            rtf: None,
            plain: Some(plain),
            source_url: None,
        });
        assert!(matches!(
            prepared,
            PreparedPaste::PlainTextFallback { ref text, .. }
                if text.as_bytes() == plain.as_bytes()
        ));
    }

    #[test]
    fn preformatted_whitespace_must_match_exactly() {
        let document =
            RichDocument::new(vec![RichBlock::CodeBlock { language: None, text: "let  x".into() }]);
        assert!(!equivalent_visible_text(&document, "let x"));
    }

    #[test]
    fn preformatted_text_cannot_match_a_later_flow_segment() {
        let document = RichDocument::new(vec![
            RichBlock::CodeBlock { language: None, text: "x  y".into() },
            RichBlock::Paragraph(vec![RichInline::Text("x  y".into())]),
        ]);

        assert!(!equivalent_visible_text(&document, "x y\nx  y"));
    }

    #[test]
    fn repeated_flow_and_preformatted_text_align_to_distinct_ranges() {
        let segments = vec![VisibleSegment::flow("same"), VisibleSegment::preformatted("same")];

        assert!(segments_align_plain(&segments, "same\nsame"));
    }

    #[test]
    fn pre_flow_pre_alignment_tries_later_repeated_candidate() {
        let segments = vec![
            VisibleSegment::preformatted("x"),
            VisibleSegment::flow("x"),
            VisibleSegment::preformatted("x"),
        ];

        assert!(segments_align_plain(&segments, "x x x"));
    }

    #[test]
    fn consecutive_flow_segments_merge_before_preformatted_alignment() {
        let segments = vec![
            VisibleSegment::flow("a"),
            VisibleSegment::flow("b"),
            VisibleSegment::preformatted("c"),
        ];

        assert!(segments_align_plain(&segments, "a\nb\nc"));
    }

    #[test]
    fn empty_segments_and_outer_whitespace_are_aligned_without_losing_bom_rules() {
        let empty_segments = vec![
            VisibleSegment::flow(""),
            VisibleSegment::preformatted(""),
            VisibleSegment::flow(""),
        ];
        assert!(segments_align_plain(&empty_segments, "\u{feff}\r\n\t"));

        let code = vec![VisibleSegment::preformatted("code")];
        assert!(segments_align_plain(&code, "\u{feff} \ncode\r\n\t"));

        let later_bom = vec![VisibleSegment::flow("a"), VisibleSegment::preformatted("b")];
        assert!(!segments_align_plain(&later_bom, "a \u{feff} b"));
    }

    #[test]
    fn flow_collapses_whitespace_while_preformatted_only_normalizes_line_endings() {
        let segments = vec![VisibleSegment::flow("a  b"), VisibleSegment::preformatted("x\r\ny")];

        assert!(segments_align_plain(&segments, "\u{feff} a b\r\nx\ny "));
        assert!(!segments_align_plain(&segments, "a b\r\nx \ny"));
    }

    #[test]
    fn repeated_empty_candidates_scale_with_streaming_alignment() {
        let mut segments = Vec::new();
        for _ in 0..1_000 {
            segments.push(VisibleSegment::preformatted(""));
            segments.push(VisibleSegment::flow(""));
        }
        segments.push(VisibleSegment::preformatted(" z "));

        let plain = format!("{}z{}", "\t".repeat(2_000), "\t".repeat(2_000));
        assert!(!segments_align_plain(&segments, &plain));
    }

    #[test]
    fn many_sibling_preformatted_segments_do_not_grow_the_call_stack() {
        let segments = (0..8_000).map(|_| VisibleSegment::preformatted("")).collect::<Vec<_>>();

        assert!(segments_align_plain(&segments, ""));
    }

    #[test]
    fn non_breaking_space_is_equivalent_in_flow_text() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::Text(
            "a\u{a0}b".into(),
        )])]);
        assert!(equivalent_visible_text(&document, "a b"));
    }

    #[test]
    fn empty_representations_are_absent_but_whitespace_markdown_is_explicit() {
        let whitespace_markdown = prepare_paste(PasteRepresentations {
            markdown: Some(" \n"),
            html: Some("<strong>ignored</strong>"),
            rtf: None,
            plain: Some("ignored"),
            source_url: None,
        });
        assert_eq!(whitespace_markdown, PreparedPaste::Markdown(" \n".into()));

        let empty_representations = prepare_paste(PasteRepresentations {
            markdown: Some(""),
            html: Some(""),
            rtf: Some(b""),
            plain: Some(""),
            source_url: None,
        });
        assert_eq!(empty_representations, PreparedPaste::Empty);
    }

    #[test]
    fn html_failure_reasons_respect_rtf_priority() {
        let invalid_html = format!(
            "{}plain{}",
            "<div>".repeat(crate::paste::html::MAX_HTML_NESTING_DEPTH + 1),
            "</div>".repeat(crate::paste::html::MAX_HTML_NESTING_DEPTH + 1),
        );
        let html_failure = prepare_paste(PasteRepresentations {
            markdown: None,
            html: Some(&invalid_html),
            rtf: None,
            plain: Some("plain"),
            source_url: None,
        });
        assert!(matches!(
            html_failure,
            PreparedPaste::PlainTextFallback { reason: PasteFallbackReason::HtmlParseFailed, .. }
        ));

        let rtf_failure = prepare_paste(PasteRepresentations {
            markdown: None,
            html: Some(&invalid_html),
            rtf: Some(br"{\rtf1 text"),
            plain: Some("plain"),
            source_url: None,
        });
        assert!(matches!(
            rtf_failure,
            PreparedPaste::PlainTextFallback { reason: PasteFallbackReason::RtfParseFailed, .. }
        ));
    }

    #[test]
    fn nonsemantic_html_without_plain_falls_through_to_rtf() {
        let prepared = prepare_paste(PasteRepresentations {
            markdown: None,
            html: Some("<span style='color:red'>rich</span>"),
            rtf: Some(br"{\rtf1\b rich\b0}"),
            plain: None,
            source_url: None,
        });
        assert_eq!(prepared, PreparedPaste::RtfConverted("**rich**".into()));
    }

    #[test]
    fn semantic_html_mismatch_does_not_continue_to_matching_rtf() {
        let prepared = prepare_paste(PasteRepresentations {
            markdown: None,
            html: Some("<strong>different</strong>"),
            rtf: Some(br"{\rtf1\b plain\b0}"),
            plain: Some("plain"),
            source_url: None,
        });
        assert!(matches!(
            prepared,
            PreparedPaste::PlainTextFallback { reason: PasteFallbackReason::TextMismatch, .. }
        ));
    }

    #[test]
    fn rtf_mismatch_uses_plain_text_with_mismatch_reason() {
        let prepared = prepare_paste(PasteRepresentations {
            markdown: None,
            html: None,
            rtf: Some(br"{\rtf1\b different\b0}"),
            plain: Some("plain"),
            source_url: None,
        });
        assert_eq!(
            prepared,
            PreparedPaste::PlainTextFallback {
                text: "plain".into(),
                reason: PasteFallbackReason::TextMismatch,
            }
        );
    }

    #[test]
    fn plain_paragraph_html_is_nonsemantic() {
        let prepared = prepare_paste(PasteRepresentations {
            markdown: None,
            html: Some("<p>plain</p>"),
            rtf: Some(br"{\rtf1\b plain\b0}"),
            plain: Some("plain"),
            source_url: None,
        });
        assert_eq!(
            prepared,
            PreparedPaste::PlainTextFallback {
                text: "plain".into(),
                reason: PasteFallbackReason::NoSemanticHtml,
            }
        );
    }

    #[test]
    fn repeated_preformatted_segments_cannot_match_overlapping_plain_ranges() {
        let document = RichDocument::new(vec![
            RichBlock::CodeBlock { language: None, text: "aba".into() },
            RichBlock::CodeBlock { language: None, text: "aba".into() },
        ]);
        assert!(!preformatted_segments_appear_in_order(&document, "ababa"));
    }

    #[test]
    fn preformatted_segments_must_match_in_document_order() {
        let document = RichDocument::new(vec![
            RichBlock::CodeBlock { language: None, text: "first".into() },
            RichBlock::CodeBlock { language: None, text: "second".into() },
        ]);
        assert!(!equivalent_visible_text(&document, "second first"));
    }

    #[test]
    fn preformatted_segments_normalize_crlf_without_collapsing_spaces() {
        let document = RichDocument::new(vec![RichBlock::CodeBlock {
            language: None,
            text: "let  x\r\nnext".into(),
        }]);
        assert!(equivalent_visible_text(&document, "let  x\nnext"));
        assert!(!equivalent_visible_text(&document, "let x\nnext"));
    }

    #[test]
    fn flow_normalization_handles_bom_unicode_whitespace_and_not_combining_forms() {
        let document = RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::Text(
            "\u{feff}a\u{2003}b\r\nc".into(),
        )])]);
        assert!(equivalent_visible_text(&document, "a b c"));

        let decomposed = RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::Text(
            "e\u{301}".into(),
        )])]);
        assert!(!equivalent_visible_text(&decomposed, "\u{e9}"));
    }

    #[test]
    fn whitespace_only_rich_writer_result_is_empty() {
        let prepared = prepare_paste(PasteRepresentations {
            markdown: None,
            html: None,
            rtf: Some(br"{\rtf1    }"),
            plain: None,
            source_url: None,
        });
        assert_eq!(prepared, PreparedPaste::Empty);
    }
}
