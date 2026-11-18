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
        let html =
            format!(r#"<script>bad()</script><img src="{source}" alt="diagram" onload="bad()">"#);
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
    assert_eq!(write_markdown(&conversion.document), "before\n\n- one\n\nmiddle\n\n- two\n\nafter");
}

#[test]
fn list_ignores_invisible_non_item_children_without_splitting_items() {
    let conversion =
        parse_html("<ul>\n <li>one</li>\n <span hidden>secret</span>\n <li>two</li>\n</ul>", None)
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

    let hidden = parse_html("<div style='display:none!important;display:block'>secret</div>", None)
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
    let conversion =
        parse_html(r#"<pre><code class="foo language-rust bar">fn main() {}</code></pre>"#, None)
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
    let conversion =
        parse_html("<script>const x = '</scripture><p>';</script><p>one<strong>two<p>three", None)
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
    let conversion =
        parse_html("<p>one<strong/>two<p>three", None).expect("non-void self-closing flag fixture");
    assert_eq!(write_markdown(&conversion.document), "one**two**\n\nthree");
}

#[test]
fn synthetic_closes_do_not_consume_same_named_outer_formatting() {
    let conversion = parse_html("<strong><p>A<strong>B<p>C</strong>D</p><p>E</p></strong>", None)
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
    let markdown = write_markdown(&plaintext.document);
    assert_eq!(markdown, "one**two**\n\nraw\\</plaintext>\\<p>three");

    let reparsed = crate::parser::parse_markdown(&markdown);
    assert!(
        reparsed
            .events
            .iter()
            .all(|event| !matches!(event, crate::parser::MarkdownEvent::InlineHtml(_)))
    );
    let visible_text: String = reparsed
        .events
        .into_iter()
        .filter_map(|event| match event {
            crate::parser::MarkdownEvent::Text(text) => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(visible_text, "onetworaw</plaintext><p>three");
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
    let conversion =
        parse_html("<p>one<strong>two<p>three</p><div hidden><strong>hidden</strong></div>", None)
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

    let table_prefix =
        parse_html("<table><tr><td><p>cell</p></td></tr></table><p>one<strong>two<p>three", None)
            .expect("table cell paragraph fixture");
    assert_eq!(write_markdown(&table_prefix.document), "| cell |\n| --- |\n\none**two**\n\nthree");
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
fn remote_images_without_alt_propagate_semantics_through_containers() {
    let fixtures = [
        "<a href='https://example.com'><img src='https://example.com/a.png'></a>",
        "<h2><img src='https://example.com/a.png'></h2>",
        "<blockquote><img src='https://example.com/a.png'></blockquote>",
        "<ul><li><img src='https://example.com/a.png'></li></ul>",
        "<table><tr><td><img src='https://example.com/a.png'></td></tr></table>",
    ];
    for html in fixtures {
        let conversion = parse_html(html, None).expect("remote image container fixture");
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
        "<br># source",
    ];
    for html in absent_fixtures {
        let conversion = parse_html(html, None).expect("empty semantic fixture");
        assert_eq!(conversion.semantic_markup, SemanticMarkup::Absent, "{html}");
    }

    let effective_break = parse_html("a<br>b", None).expect("effective break fixture");
    assert_eq!(effective_break.semantic_markup, SemanticMarkup::Present);
}
