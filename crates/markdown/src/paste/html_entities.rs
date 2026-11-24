const MAX_ENTITY_NAME_BYTES: usize = 16;
const MAX_UNICODE_SCALAR: u32 = 0x10_FFFF;
const SURROGATE_START: u32 = 0xD800;
const SURROGATE_END: u32 = 0xDFFF;
const C1_START: u32 = 0x80;
const C1_END: u32 = 0x9F;

const WINDOWS_1252_C1_MAP: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

const LEGACY_TEXT_ENTITIES: [(&str, char); 7] = [
    ("amp", '&'),
    ("copy", '\u{00A9}'),
    ("gt", '>'),
    ("lt", '<'),
    ("nbsp", '\u{00A0}'),
    ("quot", '"'),
    ("reg", '\u{00AE}'),
];

const NAMED_ENTITIES: &[(&str, char)] = &[
    ("AElig", '\u{00C6}'),
    ("Aacute", '\u{00C1}'),
    ("Acirc", '\u{00C2}'),
    ("Agrave", '\u{00C0}'),
    ("Alpha", '\u{0391}'),
    ("Aring", '\u{00C5}'),
    ("Atilde", '\u{00C3}'),
    ("Auml", '\u{00C4}'),
    ("Beta", '\u{0392}'),
    ("Ccedil", '\u{00C7}'),
    ("Chi", '\u{03A7}'),
    ("Dagger", '\u{2021}'),
    ("Delta", '\u{0394}'),
    ("ETH", '\u{00D0}'),
    ("Eacute", '\u{00C9}'),
    ("Ecirc", '\u{00CA}'),
    ("Egrave", '\u{00C8}'),
    ("Epsilon", '\u{0395}'),
    ("Eta", '\u{0397}'),
    ("Euml", '\u{00CB}'),
    ("Gamma", '\u{0393}'),
    ("Iacute", '\u{00CD}'),
    ("Icirc", '\u{00CE}'),
    ("Igrave", '\u{00CC}'),
    ("Iota", '\u{0399}'),
    ("Iuml", '\u{00CF}'),
    ("Kappa", '\u{039A}'),
    ("Lambda", '\u{039B}'),
    ("Mu", '\u{039C}'),
    ("Ntilde", '\u{00D1}'),
    ("Nu", '\u{039D}'),
    ("OElig", '\u{0152}'),
    ("Oacute", '\u{00D3}'),
    ("Ocirc", '\u{00D4}'),
    ("Ograve", '\u{00D2}'),
    ("Omega", '\u{03A9}'),
    ("Omicron", '\u{039F}'),
    ("Oslash", '\u{00D8}'),
    ("Otilde", '\u{00D5}'),
    ("Ouml", '\u{00D6}'),
    ("Phi", '\u{03A6}'),
    ("Pi", '\u{03A0}'),
    ("Prime", '\u{2033}'),
    ("Psi", '\u{03A8}'),
    ("Rho", '\u{03A1}'),
    ("Scaron", '\u{0160}'),
    ("Sigma", '\u{03A3}'),
    ("THORN", '\u{00DE}'),
    ("Tau", '\u{03A4}'),
    ("Theta", '\u{0398}'),
    ("Uacute", '\u{00DA}'),
    ("Ucirc", '\u{00DB}'),
    ("Ugrave", '\u{00D9}'),
    ("Upsilon", '\u{03A5}'),
    ("Uuml", '\u{00DC}'),
    ("Xi", '\u{039E}'),
    ("Yacute", '\u{00DD}'),
    ("Yuml", '\u{0178}'),
    ("Zeta", '\u{0396}'),
    ("aacute", '\u{00E1}'),
    ("acirc", '\u{00E2}'),
    ("acute", '\u{00B4}'),
    ("aelig", '\u{00E6}'),
    ("agrave", '\u{00E0}'),
    ("alefsym", '\u{2135}'),
    ("alpha", '\u{03B1}'),
    ("amp", '&'),
    ("and", '\u{2227}'),
    ("ang", '\u{2220}'),
    ("apos", '\''),
    ("aring", '\u{00E5}'),
    ("asymp", '\u{2248}'),
    ("atilde", '\u{00E3}'),
    ("auml", '\u{00E4}'),
    ("bdquo", '\u{201E}'),
    ("beta", '\u{03B2}'),
    ("brvbar", '\u{00A6}'),
    ("bull", '\u{2022}'),
    ("cap", '\u{2229}'),
    ("ccedil", '\u{00E7}'),
    ("cedil", '\u{00B8}'),
    ("cent", '\u{00A2}'),
    ("chi", '\u{03C7}'),
    ("circ", '\u{02C6}'),
    ("clubs", '\u{2663}'),
    ("cong", '\u{2245}'),
    ("copy", '\u{00A9}'),
    ("crarr", '\u{21B5}'),
    ("cup", '\u{222A}'),
    ("curren", '\u{00A4}'),
    ("dArr", '\u{21D3}'),
    ("dagger", '\u{2020}'),
    ("darr", '\u{2193}'),
    ("deg", '\u{00B0}'),
    ("delta", '\u{03B4}'),
    ("diams", '\u{2666}'),
    ("divide", '\u{00F7}'),
    ("eacute", '\u{00E9}'),
    ("ecirc", '\u{00EA}'),
    ("egrave", '\u{00E8}'),
    ("empty", '\u{2205}'),
    ("emsp", '\u{2003}'),
    ("ensp", '\u{2002}'),
    ("epsilon", '\u{03B5}'),
    ("equiv", '\u{2261}'),
    ("eta", '\u{03B7}'),
    ("eth", '\u{00F0}'),
    ("euml", '\u{00EB}'),
    ("euro", '\u{20AC}'),
    ("exist", '\u{2203}'),
    ("fnof", '\u{0192}'),
    ("forall", '\u{2200}'),
    ("frac12", '\u{00BD}'),
    ("frac14", '\u{00BC}'),
    ("frac34", '\u{00BE}'),
    ("frasl", '\u{2044}'),
    ("gamma", '\u{03B3}'),
    ("ge", '\u{2265}'),
    ("gt", '>'),
    ("hArr", '\u{21D4}'),
    ("harr", '\u{2194}'),
    ("hearts", '\u{2665}'),
    ("hellip", '\u{2026}'),
    ("iacute", '\u{00ED}'),
    ("icirc", '\u{00EE}'),
    ("iexcl", '\u{00A1}'),
    ("igrave", '\u{00EC}'),
    ("image", '\u{2111}'),
    ("infin", '\u{221E}'),
    ("int", '\u{222B}'),
    ("iota", '\u{03B9}'),
    ("iquest", '\u{00BF}'),
    ("isin", '\u{2208}'),
    ("iuml", '\u{00EF}'),
    ("kappa", '\u{03BA}'),
    ("lArr", '\u{21D0}'),
    ("lambda", '\u{03BB}'),
    ("lang", '\u{27E8}'),
    ("laquo", '\u{00AB}'),
    ("larr", '\u{2190}'),
    ("lceil", '\u{2308}'),
    ("ldquo", '\u{201C}'),
    ("le", '\u{2264}'),
    ("lfloor", '\u{230A}'),
    ("lowast", '\u{2217}'),
    ("loz", '\u{25CA}'),
    ("lrm", '\u{200E}'),
    ("lsaquo", '\u{2039}'),
    ("lsquo", '\u{2018}'),
    ("lt", '<'),
    ("macr", '\u{00AF}'),
    ("mdash", '\u{2014}'),
    ("micro", '\u{00B5}'),
    ("middot", '\u{00B7}'),
    ("minus", '\u{2212}'),
    ("mu", '\u{03BC}'),
    ("nabla", '\u{2207}'),
    ("nbsp", '\u{00A0}'),
    ("ndash", '\u{2013}'),
    ("ne", '\u{2260}'),
    ("ni", '\u{220B}'),
    ("not", '\u{00AC}'),
    ("notin", '\u{2209}'),
    ("nsub", '\u{2284}'),
    ("ntilde", '\u{00F1}'),
    ("nu", '\u{03BD}'),
    ("oacute", '\u{00F3}'),
    ("ocirc", '\u{00F4}'),
    ("oelig", '\u{0153}'),
    ("ograve", '\u{00F2}'),
    ("oline", '\u{203E}'),
    ("omega", '\u{03C9}'),
    ("omicron", '\u{03BF}'),
    ("oplus", '\u{2295}'),
    ("or", '\u{2228}'),
    ("ordf", '\u{00AA}'),
    ("ordm", '\u{00BA}'),
    ("oslash", '\u{00F8}'),
    ("otilde", '\u{00F5}'),
    ("otimes", '\u{2297}'),
    ("ouml", '\u{00F6}'),
    ("para", '\u{00B6}'),
    ("part", '\u{2202}'),
    ("permil", '\u{2030}'),
    ("perp", '\u{22A5}'),
    ("phi", '\u{03C6}'),
    ("pi", '\u{03C0}'),
    ("piv", '\u{03D6}'),
    ("plusmn", '\u{00B1}'),
    ("pound", '\u{00A3}'),
    ("prime", '\u{2032}'),
    ("prod", '\u{220F}'),
    ("prop", '\u{221D}'),
    ("psi", '\u{03C8}'),
    ("quot", '"'),
    ("rArr", '\u{21D2}'),
    ("radic", '\u{221A}'),
    ("rang", '\u{27E9}'),
    ("raquo", '\u{00BB}'),
    ("rarr", '\u{2192}'),
    ("rceil", '\u{2309}'),
    ("rdquo", '\u{201D}'),
    ("real", '\u{211C}'),
    ("reg", '\u{00AE}'),
    ("rfloor", '\u{230B}'),
    ("rho", '\u{03C1}'),
    ("rlm", '\u{200F}'),
    ("rsaquo", '\u{203A}'),
    ("rsquo", '\u{2019}'),
    ("sbquo", '\u{201A}'),
    ("scaron", '\u{0161}'),
    ("sdot", '\u{22C5}'),
    ("sect", '\u{00A7}'),
    ("shy", '\u{00AD}'),
    ("sigma", '\u{03C3}'),
    ("sigmaf", '\u{03C2}'),
    ("sim", '\u{223C}'),
    ("spades", '\u{2660}'),
    ("sub", '\u{2282}'),
    ("sube", '\u{2286}'),
    ("sum", '\u{2211}'),
    ("sup", '\u{2283}'),
    ("sup1", '\u{00B9}'),
    ("sup2", '\u{00B2}'),
    ("sup3", '\u{00B3}'),
    ("supe", '\u{2287}'),
    ("szlig", '\u{00DF}'),
    ("tau", '\u{03C4}'),
    ("there4", '\u{2234}'),
    ("theta", '\u{03B8}'),
    ("thetasym", '\u{03D1}'),
    ("thinsp", '\u{2009}'),
    ("thorn", '\u{00FE}'),
    ("tilde", '\u{02DC}'),
    ("times", '\u{00D7}'),
    ("trade", '\u{2122}'),
    ("uArr", '\u{21D1}'),
    ("uacute", '\u{00FA}'),
    ("uarr", '\u{2191}'),
    ("ucirc", '\u{00FB}'),
    ("ugrave", '\u{00F9}'),
    ("uml", '\u{00A8}'),
    ("upsih", '\u{03D2}'),
    ("upsilon", '\u{03C5}'),
    ("uuml", '\u{00FC}'),
    ("weierp", '\u{2118}'),
    ("xi", '\u{03BE}'),
    ("yacute", '\u{00FD}'),
    ("yen", '\u{00A5}'),
    ("yuml", '\u{00FF}'),
    ("zeta", '\u{03B6}'),
    ("zwj", '\u{200D}'),
    ("zwnj", '\u{200C}'),
];

pub(super) fn decode_html_entities(input: &str, in_attribute: bool) -> String {
    if !input.contains('&') {
        return input.to_owned();
    }

    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;

    while let Some(offset) = bytes[cursor..].iter().position(|byte| *byte == b'&') {
        let amp_index = cursor + offset;
        output.push_str(&input[cursor..amp_index]);
        if let Some((decoded, next_cursor)) = parse_entity(input, amp_index, in_attribute) {
            output.push(decoded);
            cursor = next_cursor;
        } else {
            output.push('&');
            cursor = amp_index + 1;
        }
    }

    output.push_str(&input[cursor..]);
    output
}

fn parse_entity(input: &str, amp_index: usize, in_attribute: bool) -> Option<(char, usize)> {
    let bytes = input.as_bytes();
    if bytes.get(amp_index + 1) == Some(&b'#') {
        return parse_numeric_entity(bytes, amp_index + 2);
    }
    parse_named_entity(input, amp_index + 1, in_attribute)
}

fn parse_numeric_entity(bytes: &[u8], start: usize) -> Option<(char, usize)> {
    let (radix, digit_start) = match bytes.get(start) {
        Some(b'x' | b'X') => (16_u32, start + 1),
        _ => (10_u32, start),
    };
    let mut cursor = digit_start;
    let mut code = 0_u32;
    let mut has_digits = false;

    while let Some(&byte) = bytes.get(cursor) {
        let Some(digit) = (byte as char).to_digit(radix) else {
            break;
        };
        has_digits = true;
        code = code.saturating_mul(radix).saturating_add(digit);
        cursor += 1;
    }

    if !has_digits {
        return None;
    }
    if bytes.get(cursor) == Some(&b';') {
        cursor += 1;
    }
    Some((decode_numeric_codepoint(code), cursor))
}

fn decode_numeric_codepoint(code: u32) -> char {
    if code == 0 || (SURROGATE_START..=SURROGATE_END).contains(&code) || code > MAX_UNICODE_SCALAR {
        return '\u{FFFD}';
    }
    if (C1_START..=C1_END).contains(&code) {
        return WINDOWS_1252_C1_MAP[(code - C1_START) as usize];
    }
    char::from_u32(code).unwrap_or('\u{FFFD}')
}

fn parse_named_entity(input: &str, name_start: usize, in_attribute: bool) -> Option<(char, usize)> {
    let bytes = input.as_bytes();
    let mut cursor = name_start;
    while bytes.get(cursor).is_some_and(u8::is_ascii_alphanumeric) {
        cursor += 1;
        if cursor - name_start > MAX_ENTITY_NAME_BYTES {
            return None;
        }
    }
    if cursor == name_start {
        return None;
    }

    let name = &input[name_start..cursor];
    if bytes.get(cursor) == Some(&b';') {
        let index = NAMED_ENTITIES.binary_search_by_key(&name, |(key, _)| *key).ok()?;
        return Some((NAMED_ENTITIES[index].1, cursor + 1));
    }

    if in_attribute {
        return None;
    }
    let index = LEGACY_TEXT_ENTITIES.binary_search_by_key(&name, |(key, _)| *key).ok()?;
    Some((LEGACY_TEXT_ENTITIES[index].1, cursor))
}

#[cfg(test)]
mod tests {
    use super::{LEGACY_TEXT_ENTITIES, NAMED_ENTITIES, decode_html_entities};

    #[test]
    fn entity_tables_are_strictly_sorted_for_binary_search() {
        for pair in NAMED_ENTITIES.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "NAMED_ENTITIES must be strictly sorted: {:?} >= {:?}",
                pair[0].0,
                pair[1].0
            );
        }
        for pair in LEGACY_TEXT_ENTITIES.windows(2) {
            assert!(
                pair[0].0 < pair[1].0,
                "LEGACY_TEXT_ENTITIES must be strictly sorted: {:?} >= {:?}",
                pair[0].0,
                pair[1].0
            );
        }
    }

    #[test]
    fn decodes_named_and_numeric_entities() {
        assert_eq!(
            decode_html_entities("a &amp; b &lt; c &gt; &quot;d&quot;", false),
            "a & b < c > \"d\""
        );
        assert_eq!(
            decode_html_entities("&mdash; &ldquo;hi&rdquo; &copy; &#65; &#x1F600;", false),
            "— “hi” © A 😀"
        );
        assert_eq!(decode_html_entities("&#0000000065;&#x0000000041;&#X42", false), "AAB");
    }

    #[test]
    fn maps_c1_controls_and_rejects_invalid_codepoints() {
        assert_eq!(decode_html_entities("&#128;&#147;&#151;", false), "€\u{201C}—");
        assert_eq!(
            decode_html_entities("&#0;&#xD800;&#x110000;&#x100000000;", false),
            "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}"
        );
    }

    #[test]
    fn preserves_unsemicoloned_entities_in_attribute_urls() {
        assert_eq!(
            decode_html_entities("https://example.com/?a=1&copy=2&reg=3&amp;b=4", true),
            "https://example.com/?a=1&copy=2&reg=3&b=4"
        );
        assert_eq!(decode_html_entities("a &copy b &copy1", false), "a © b &copy1");
    }
}
