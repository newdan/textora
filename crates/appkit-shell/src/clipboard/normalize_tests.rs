use super::*;

#[test]
fn normalize_strips_bom() {
    let input = b"\xEF\xBB\xBFhello";
    let result = normalize_paste_text(input);
    assert_eq!(result, &b"hello"[..]);
}

#[test]
fn normalize_no_bom_passthrough() {
    let input = b"hello world";
    let result = normalize_paste_text(input);
    assert_eq!(result, &b"hello world"[..]);
}

#[test]
fn normalize_bom_in_middle_preserved() {
    let input = b"hello\xEF\xBB\xBFworld";
    let result = normalize_paste_text(input);
    assert_eq!(result, b"hello\xEF\xBB\xBFworld", "BOM in middle should be preserved");
}

#[test]
fn normalize_crlf_to_lf() {
    let input = b"line1\r\nline2\r\nline3";
    let result = normalize_paste_text(input);
    assert_eq!(result, &b"line1\nline2\nline3"[..]);
}

#[test]
fn normalize_bare_cr_to_lf() {
    let input = b"line1\rline2";
    let result = normalize_paste_text(input);
    assert_eq!(result, &b"line1\nline2"[..]);
}

#[test]
fn normalize_bom_plus_crlf() {
    let input = b"\xEF\xBB\xBFline1\r\nline2";
    let result = normalize_paste_text(input);
    assert_eq!(result, &b"line1\nline2"[..]);
}

#[test]
fn normalize_empty_input() {
    let result = normalize_paste_text(b"");
    assert!(result.is_empty());
}
