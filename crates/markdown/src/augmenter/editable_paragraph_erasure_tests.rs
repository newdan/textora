use super::*;

fn apply_erasure(source: &str, content: &str) -> String {
    let start = source.find(content).expect("fixture contains selected content");
    let edit = erase_range(source, start..start + content.len())
        .expect("clearing a complete paragraph must produce an edit");
    let mut edited = source.to_string();
    edited.replace_range(
        edit.replace_range.expect("paragraph erasure provides its replacement range"),
        &edit.insert_text.expect("paragraph erasure provides replacement text"),
    );
    assert!(edited.is_char_boundary(edit.cursor_byte_after));
    edited
}

#[test]
fn paragraph_range_erasure_keeps_one_slot_across_block_neighbors() {
    for newline in ["\n", "\r\n"] {
        for before in
            ["a", "# heading", "---", "- item", "> quote", "```\ncode\n```", "| a |\n| - |"]
        {
            for after in
                ["b", "# heading", "---", "- item", "> quote", "```\ncode\n```", "| a |\n| - |"]
            {
                let source = format!("{before}{newline}{newline}words{newline}{newline}{after}");
                assert_eq!(
                    apply_erasure(&source, "words"),
                    format!("{before}{newline}{newline}{newline}{after}"),
                    "{source:?}"
                );
            }
        }
    }
}

#[test]
fn paragraph_range_erasure_preserves_edges_and_adjacent_slots() {
    for (source, expected) in [
        ("words", ""),
        ("words\n\nb", "\nb"),
        ("a\n\nwords", "a\n\n"),
        ("a\n\nwords\n", "a\n\n\n"),
        ("a\n\n\nwords\n\n\n\nb", "a\n\n\n\n\n\nb"),
        ("> a\n>\n> words\n>\n> b", "> a\n>\n> \n> b"),
        ("- a\n\n  words\n\n  b", "- a\n\n  \n  b"),
        ("> a\r\n>\r\n> words\r\n>\r\n> b", "> a\r\n>\r\n> \r\n> b"),
    ] {
        assert_eq!(apply_erasure(source, "words"), expected, "{source:?}");
    }
}

#[test]
fn paragraph_range_erasure_removes_styles_and_unicode_content() {
    for content in ["**words**", "*words*", "[words](https://example.test)", "👩‍💻", "e\u{301}"]
    {
        let source = format!("a\n\n{content}\n\nb");
        let selected = if content.contains("words") { "words" } else { content };
        assert_eq!(apply_erasure(&source, selected), "a\n\n\nb", "{source:?}");
    }
}

#[test]
fn paragraph_range_erasure_rejects_partial_atomic_and_invalid_ranges() {
    for (source, erased) in [
        ("words", 0..1),
        ("`x`", 1..2),
        ("```\nwords\n```", 4..9),
        ("a\n\n---\n\nb", 0..10),
        ("é", 0..1),
        ("a", 0..2),
        ("a", std::ops::Range { start: 1, end: 0 }),
    ] {
        assert!(erase_range(source, erased).is_none(), "{source:?}");
    }
}

#[test]
fn paragraph_grapheme_erasure_matches_range_erasure() {
    for content in ["x", "👩‍💻", "e\u{301}"] {
        let source = format!("a\n\n{content}\n\nb");
        let end = 3 + content.len();
        let edit = erase_last_grapheme(&source, end).expect("last grapheme empties paragraph");
        let mut edited = source.clone();
        edited.replace_range(
            edit.replace_range.expect("replacement range"),
            &edit.insert_text.expect("replacement text"),
        );
        assert_eq!(edited, apply_erasure(&source, content));
    }
}

#[test]
fn paragraph_grapheme_erasure_handles_caret_before_closing_styles() {
    for content in ["**x**", "*x*", "~~x~~", "[x](https://example.test)"] {
        let source = format!("a\n\n{content}\n\nb");
        let cursor = source.find('x').expect("fixture contains visible grapheme") + 1;
        let edit = erase_last_grapheme(&source, cursor)
            .expect("visible final grapheme empties styled paragraph");
        let mut edited = source.clone();
        edited.replace_range(
            edit.replace_range.expect("replacement range"),
            &edit.insert_text.expect("replacement text"),
        );
        assert_eq!(edited, "a\n\n\nb", "{source:?}");
    }
}

#[test]
fn paragraph_range_erasure_clears_multiline_content_inside_its_owner() {
    for (source, content, expected) in [
        ("a\n\nfirst\nsecond\n\nb", "first\nsecond", "a\n\n\nb"),
        ("> a\n>\n> first\n> second\n>\n> b", "first\n> second", "> a\n>\n> \n> b"),
    ] {
        assert_eq!(apply_erasure(source, content), expected);
    }
}

#[test]
fn paragraph_range_erasure_removes_complete_multiline_link_syntax() {
    for newline in ["\n", "\r\n"] {
        for link in ["[x](\nurl\n)", "[x](url\n\"title\")", "[x](url\n\"first\nsecond\"\n)"] {
            let link = link.replace('\n', newline);
            let source = format!("a{newline}{newline}{link}{newline}{newline}b");
            assert_eq!(
                apply_erasure(&source, "x"),
                format!("a{newline}{newline}{newline}b"),
                "{source:?}"
            );
            let quoted_link = link.replace(newline, &format!("{newline}> "));
            let quoted = format!("> a{newline}>{newline}> {quoted_link}{newline}>{newline}> b");
            assert_eq!(
                apply_erasure(&quoted, "x"),
                format!("> a{newline}>{newline}> {newline}> b"),
                "{quoted:?}"
            );
            let indented_link = link.replace(newline, &format!("{newline}  "));
            let listed = format!("- a{newline}{newline}  {indented_link}{newline}{newline}  b");
            assert_eq!(
                apply_erasure(&listed, "x"),
                format!("- a{newline}{newline}  {newline}  b"),
                "{listed:?}"
            );
            assert_eq!(apply_erasure(&link, "x"), "", "{link:?}");
        }
    }
}
