use super::{
    MAX_RTF_CONTROL_WORD_BYTES, MAX_RTF_GROUP_DEPTH, RtfPasteError, parse_rtf,
    safe_hyperlink_destination,
};
use crate::paste::writer::write_markdown;

#[test]
fn parses_paragraphs_unicode_inline_styles_and_hyperlinks() {
    let input = br#"{\rtf1\ansi\ansicpg1252
        First \b bold\b0 \i italic\i0\par
        Unicode \u20320?\u22909? \field{\*\fldinst HYPERLINK "https://example.com"}{\fldrslt link}}
    }"#;
    let document = parse_rtf(input).expect("supported RTF fixture");
    assert_eq!(
        write_markdown(&document),
        "First **bold** *italic*\n\nUnicode 你好 [link](https://example.com/)"
    );
}

#[test]
fn excessive_group_depth_returns_a_typed_error() {
    let input = "{".repeat(MAX_RTF_GROUP_DEPTH + 1);
    assert_eq!(parse_rtf(input.as_bytes()), Err(RtfPasteError::GroupDepthExceeded));
}

#[test]
fn exact_group_and_control_word_limits_are_accepted() {
    let groups = "{".repeat(MAX_RTF_GROUP_DEPTH);
    let closes = "}".repeat(MAX_RTF_GROUP_DEPTH);
    let document = parse_rtf(format!("{groups}x{closes}").as_bytes())
        .expect("group depth at the limit is valid");
    assert_eq!(write_markdown(&document), "x");

    let control_word = "a".repeat(MAX_RTF_CONTROL_WORD_BYTES);
    parse_rtf(format!("{{\\{control_word}}}").as_bytes())
        .expect("control word length at the limit is valid");
}

#[test]
fn cells_and_rows_degrade_to_ordered_visible_text() {
    let document = parse_rtf(br"{\rtf1 a\cell b\cell\row}").expect("table controls degrade safely");
    assert_eq!(write_markdown(&document), "a\tb");
}

#[test]
fn malformed_tokens_return_typed_errors() {
    assert_eq!(parse_rtf(br"{\rtf1 text"), Err(RtfPasteError::UnclosedGroup));
    assert_eq!(parse_rtf(br"{\rtf1 \'zz}"), Err(RtfPasteError::InvalidHexEscape));
    assert_eq!(parse_rtf(br"}"), Err(RtfPasteError::UnmatchedGroupEnd));
    assert_eq!(parse_rtf(br"{\rtf1 \"), Err(RtfPasteError::DanglingBackslash));
}

#[test]
fn oversized_control_words_and_arguments_return_typed_errors() {
    let oversized_word = "a".repeat(MAX_RTF_CONTROL_WORD_BYTES + 1);
    let input = format!("{{\\{oversized_word}}}");
    assert_eq!(parse_rtf(input.as_bytes()), Err(RtfPasteError::ControlWordTooLong));
    assert_eq!(parse_rtf(br"{\rtf2147483648}"), Err(RtfPasteError::InvalidControlArgument));
}

#[test]
fn signed_and_large_supported_arguments_are_not_truncated() {
    let document = parse_rtf(br"{\rtf1\b-1 bold\b0\uc2147483647\u65?}")
        .expect("in-range signed arguments remain supported");
    assert_eq!(write_markdown(&document), "**bold**A");
}

#[test]
fn unicode_fallback_and_strike_are_decoded_once() {
    let document = parse_rtf(br"{\rtf1\uc1 \u-10180?\u-8435? \strike gone\strike0}")
        .expect("signed UTF-16 units and strike are supported");
    assert_eq!(write_markdown(&document), "🌍 ~~gone~~");
}

#[test]
fn unicode_fallback_counts_text_and_control_tokens_exactly() {
    let document = parse_rtf(br"{\rtf1\uc0\u65? \uc2\u66\'3f?X \uc1\u67\tab Y}")
        .expect("fallback tokens are bounded by uc");
    assert_eq!(write_markdown(&document), "A? BX CY");
}

#[test]
fn invalid_surrogate_order_returns_a_typed_error() {
    assert_eq!(parse_rtf(br"{\rtf1\u-8435?}"), Err(RtfPasteError::InvalidUnicodeSurrogateOrder));
    assert_eq!(parse_rtf(br"{\rtf1\u-10180?x}"), Err(RtfPasteError::InvalidUnicodeSurrogateOrder));
    assert_eq!(parse_rtf(br"{\rtf1\u70000?}"), Err(RtfPasteError::InvalidUnicodeCodeUnit(70000)));
    assert_eq!(
        parse_rtf(br"{\rtf1\uc-1 text}"),
        Err(RtfPasteError::InvalidUnicodeFallbackCount(-1))
    );
}

#[test]
fn surrogate_pairs_cannot_cross_semantic_or_group_boundaries() {
    assert_eq!(
        parse_rtf(br"{\rtf1\u-10180?\b\u-8435?}"),
        Err(RtfPasteError::InvalidUnicodeSurrogateOrder)
    );
    assert_eq!(
        parse_rtf(br"{\rtf1\u-10180?{\u-8435?}}"),
        Err(RtfPasteError::InvalidUnicodeSurrogateOrder)
    );
    assert_eq!(
        parse_rtf(br"{\rtf1\u-10180?\fldinst\u-8435?}"),
        Err(RtfPasteError::InvalidUnicodeSurrogateOrder)
    );
}

#[test]
fn unicode_fallback_does_not_escape_its_group() {
    let document = parse_rtf(br"{\rtf1\uc1 {\u65}B}")
        .expect("missing fallback bytes do not consume parent text");
    assert_eq!(write_markdown(&document), "AB");
}

#[test]
fn nested_groups_restore_styles_and_escaped_literals() {
    let document = parse_rtf(br"{\rtf1\b outer {\i both} outer\b0 \{x\}\\y}")
        .expect("groups inherit and restore their parent state");
    assert_eq!(write_markdown(&document), "**outer *both* outer**{x}\\\\y");
}

#[test]
fn cp1252_bytes_decode_and_unsupported_code_pages_are_typed() {
    let document = parse_rtf(br"{\rtf1\ansicpg1252 euro \'80}").expect("Windows-1252 is supported");
    assert_eq!(write_markdown(&document), "euro €");
    assert_eq!(
        parse_rtf(br"{\rtf1\ansicpg932 text}"),
        Err(RtfPasteError::UnsupportedCodePage(932))
    );
    let ansi_fallback = parse_rtf(br"{\rtf1\ansi\ansicpg932 \'80}")
        .expect("generic ANSI permits the documented Windows-1252 fallback");
    assert_eq!(write_markdown(&ansi_fallback), "€");
}

#[test]
fn field_instruction_quotes_and_unsafe_schemes_degrade_safely() {
    let safe = parse_rtf(
        br#"{\rtf1{\field{\*\fldinst HYPERLINK "https://example.com/a?q=1"}{\fldrslt safe}}}"#,
    )
    .expect("quoted safe hyperlink instruction");
    assert_eq!(write_markdown(&safe), "[safe](https://example.com/a?q=1)");
    let unsafe_field = parse_rtf(
        br#"{\rtf1{\field{\*\fldinst HYPERLINK "javascript:alert(1)"}{\fldrslt shown}}}"#,
    )
    .expect("unsafe schemes degrade to visible result text");
    assert_eq!(write_markdown(&unsafe_field), "shown");
}

#[test]
fn hyperlink_destinations_are_canonical_and_control_free() {
    assert_eq!(
        safe_hyperlink_destination(r#"HYPERLINK "https://example.com/a b>c""#),
        Some("https://example.com/a%20b%3Ec".into())
    );
    assert_eq!(
        safe_hyperlink_destination(r#"HYPERLINK "mailto:user@example.com""#),
        Some("mailto:user@example.com".into())
    );
    assert_eq!(safe_hyperlink_destination(r#"HYPERLINK "javascript:alert(1)""#), None);
    for control in ['\0', '\t', '\n', '\r'] {
        let instruction = format!("HYPERLINK \"https://example.com/a{control}b\"");
        assert_eq!(safe_hyperlink_destination(&instruction), None);
    }

    let unicode = parse_rtf(
        br#"{\rtf1{\field{\*\fldinst HYPERLINK "https://example.com/\u36335?\u24452?"}{\fldrslt unicode}}}"#,
    )
    .expect("Unicode destinations are represented canonically");
    assert_eq!(write_markdown(&unicode), "[unicode](https://example.com/%E8%B7%AF%E5%BE%84)");

    let controls = parse_rtf(
        br#"{\rtf1{\field{\*\fldinst HYPERLINK "https://example.com/\tab>injected"}{\fldrslt tab}} {\field{\*\fldinst HYPERLINK "https://example.com/\par injected"}{\fldrslt paragraph}}}"#,
    )
    .expect("unsafe field instructions degrade to their visible labels");
    assert_eq!(write_markdown(&controls), "tab paragraph");
}

#[test]
fn field_group_whitespace_does_not_change_brace_validation() {
    let document = parse_rtf(
        br#"{\rtf1 { \field{\*\fldinst HYPERLINK "https://example.com"}{\fldrslt link}}}"#,
    )
    .expect("whitespace before a grouped field is harmless");
    assert_eq!(write_markdown(&document), " [link](https://example.com/)");
    assert_eq!(parse_rtf(br"\field}"), Err(RtfPasteError::UnmatchedGroupEnd));
    assert_eq!(parse_rtf(br"x\field}"), Err(RtfPasteError::UnmatchedGroupEnd));
    assert_eq!(parse_rtf(br"{\rtf1 x\field}}"), Err(RtfPasteError::UnmatchedGroupEnd));
}

#[test]
fn completed_child_groups_advance_the_parent_group_position() {
    for child in ["{}", r"{\pict ignored}", "{visible}"] {
        let input = format!(
            r#"{{{child}\field{{\*\fldinst HYPERLINK "https://example.com"}}{{\fldrslt link}}}}"#,
        );
        assert_eq!(
            parse_rtf(input.as_bytes()),
            Err(RtfPasteError::UnclosedGroup),
            "child group must prevent grouped-field classification: {child}"
        );
    }
}

#[test]
fn adjacent_styles_do_not_invent_visible_spaces() {
    let document =
        parse_rtf(br"{\rtf1\b A\b0\i B\i0}").expect("adjacent style controls are supported");
    assert_eq!(document.visible_segments()[0].text, "AB");
    let plain = parse_rtf(br"{\rtf1\b bold\b0 plain}")
        .expect("a control-word delimiter is not visible text");
    assert_eq!(plain.visible_segments()[0].text, "boldplain");
    let explicit_space =
        parse_rtf(br"{\rtf1\b bold\b0  plain}").expect("the second of two spaces remains visible");
    assert_eq!(explicit_space.visible_segments()[0].text, "bold plain");
    let fallback = parse_rtf(br"{\rtf1\uc1\u65\b0 ?}")
        .expect("style delimiters do not consume Unicode fallback");
    assert_eq!(fallback.visible_segments()[0].text, "A");
}

#[test]
fn incomplete_implicit_fields_return_typed_errors() {
    assert_eq!(
        parse_rtf(br#"{\rtf1 x\field{\*\fldinst HYPERLINK "https://example.com"}}"#),
        Err(RtfPasteError::UnclosedGroup)
    );
    assert_eq!(parse_rtf(br"{\rtf1 x\field{\fldrslt shown}}"), Err(RtfPasteError::UnclosedGroup));
    assert_eq!(parse_rtf(br"x\field{\fldrslt shown}"), Err(RtfPasteError::UnclosedGroup));
    assert_eq!(
        parse_rtf(
            br#"x\field{\*\fldinst HYPERLINK "https://one.example"}{\fldrslt one} y\field{\*\fldinst HYPERLINK "https://two.example"}{\fldrslt two}"#,
        ),
        Err(RtfPasteError::UnclosedGroup)
    );
}

#[test]
fn nested_fields_preserve_outer_result_order() {
    let document = parse_rtf(
        br#"{\rtf1{\field{\*\fldinst HYPERLINK "javascript:outer"}{\fldrslt outer {\field{\*\fldinst HYPERLINK "https://inner.example"}{\fldrslt inner}} tail}}}"#,
    )
    .expect("nested fields use independent contexts");
    assert_eq!(write_markdown(&document), "outer [inner](https://inner.example/) tail");
}

#[test]
fn field_destinations_require_independent_closed_groups() {
    let same_group = br#"{\rtf1{\field{\*\fldinst HYPERLINK "https://example.com"\fldrslt bad}}}"#;
    assert!(parse_rtf(same_group).is_err());

    let wrong_order =
        br#"{\rtf1{\field{\fldrslt bad}{\*\fldinst HYPERLINK "https://example.com"}}}"#;
    assert!(parse_rtf(wrong_order).is_err());

    let ungrouped = br#"{\rtf1{\field\fldinst HYPERLINK "https://example.com"\fldrslt bad}}"#;
    assert!(parse_rtf(ungrouped).is_err());

    let nested_instruction = parse_rtf(
        br#"{\rtf1{\field{\*\fldinst HYPERLINK {"https://example.com/a"}}{\fldrslt link}}}"#,
    )
    .expect("nested ordinary groups stay inside the instruction group");
    assert_eq!(write_markdown(&nested_instruction), "[link](https://example.com/a)");
}

#[test]
fn field_instruction_table_controls_do_not_pollute_visible_output() {
    let document = parse_rtf(
        br#"{\rtf1 before {\field{\*\fldinst HYPERLINK "https://example.com"\cell\row}{\fldrslt link}} after}"#,
    )
    .expect("field instructions are isolated from visible structure");
    assert_eq!(write_markdown(&document), "before [link](https://example.com/) after");
}

#[test]
fn unknown_starred_destinations_and_empty_groups_are_ignored() {
    let document =
        parse_rtf(br"{\rtf1{}\par first{\*\unknown secret}\par\par second{\pict ignored}}")
            .expect("unknown destinations and empty paragraphs are harmless");
    assert_eq!(write_markdown(&document), "first\n\nsecond");
}

#[test]
fn list_destinations_and_pictures_degrade_safely() {
    let document = parse_rtf(br"{\rtf1{\pntext\'b7\tab}item\par{\pict ignored}after}")
        .expect("simple bullet and skipped picture are supported");
    assert_eq!(write_markdown(&document), "- item\n\nafter");
}

#[test]
fn nested_list_destinations_cannot_pollute_the_parent_context() {
    let instruction = parse_rtf(
        br#"{\rtf1{\field{\*\fldinst HYPERLINK "https://example.com"{\listtext 1.\tab}}{\fldrslt link}}}"#,
    )
    .expect("list markers in field instructions are isolated");
    assert_eq!(write_markdown(&instruction), "[link](https://example.com/)");

    let result = parse_rtf(
        br#"{\rtf1{\field{\*\fldinst HYPERLINK "javascript:unsafe"}{\fldrslt before {\pntext 1.\tab}label}}}"#,
    )
    .expect("list markers in field results do not change the outer paragraph");
    assert_eq!(write_markdown(&result), "before label");

    let skipped =
        parse_rtf(br#"{\rtf1{\pict{\listtext 1.\tab}ignored}{\*\unknown{\pntext 2.\tab}}plain}"#)
            .expect("skipped destinations cannot emit list markers");
    assert_eq!(write_markdown(&skipped), "plain");
}

#[test]
fn field_container_sibling_list_markers_are_suppressed() {
    for destination in ["listtext", "pntext"] {
        let input = format!(
            r#"{{\rtf1 before {{\field{{\{destination} 1.\tab}}{{\*\fldinst HYPERLINK "https://example.com"}}{{\fldrslt link}}}}\par after}}"#,
        );
        let document = parse_rtf(input.as_bytes())
            .expect("field-container sibling markers are accepted but isolated");
        assert_eq!(write_markdown(&document), "before [link](https://example.com/)\n\nafter");
    }

    let nested = parse_rtf(
        br#"{\rtf1{\field{\*\fldinst HYPERLINK "javascript:outer"}{\fldrslt outer {\field{\pntext 1.\tab}{\*\fldinst HYPERLINK "javascript:inner"}{\fldrslt inner}} tail}}}"#,
    )
    .expect("nested field-container sibling markers remain field-local");
    assert_eq!(write_markdown(&nested), "outer inner tail");
}

#[test]
fn document_list_marker_before_a_field_remains_effective() {
    let document = parse_rtf(
        br#"{\rtf1{\listtext 1.\tab}before {\field{\*\fldinst HYPERLINK "https://example.com"}{\fldrslt link}}\par after}"#,
    )
    .expect("a document list marker remains effective when followed by a field");
    assert_eq!(write_markdown(&document), "1. before [link](https://example.com/)\n\nafter");
}

#[test]
fn consecutive_ordered_list_markers_form_one_list() {
    let document = parse_rtf(br"{\rtf1{\listtext 3.\tab}third\par{\listtext 4.\tab}fourth}")
        .expect("consecutive ordered markers are supported");
    assert_eq!(write_markdown(&document), "3. third\n4. fourth");
}

#[test]
fn line_tabs_and_multiple_rows_keep_visible_order() {
    let document = parse_rtf(br"{\rtf1 a\line b\par c\tab d\cell e\row}")
        .expect("visible structural controls are supported");
    assert_eq!(write_markdown(&document), "a  \nb\n\nc\td\te");
}
