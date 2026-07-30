/// Parse C-style escape sequences in a search or replace string.
/// Supported: \n, \t, \r, \\
/// Handles multi-byte UTF-8 correctly by operating on char boundaries.
pub fn parse_escapes(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Truncate a string to `max_chars` characters, appending "..." if truncated.
/// Operates on char boundaries (UTF-8 safe).
pub fn truncate_display(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max_chars).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_newline() {
        assert_eq!(parse_escapes(r"hello\nworld"), "hello\nworld");
    }

    #[test]
    fn parse_tab() {
        assert_eq!(parse_escapes(r"a\tb"), "a\tb");
    }

    #[test]
    fn parse_backslash() {
        assert_eq!(parse_escapes(r"path\\to"), "path\\to");
    }

    #[test]
    fn parse_multiple() {
        assert_eq!(parse_escapes(r"line1\nline2\tindented"), "line1\nline2\tindented");
    }

    #[test]
    fn parse_no_escapes() {
        assert_eq!(parse_escapes("plain"), "plain");
    }

    #[test]
    fn parse_empty() {
        assert_eq!(parse_escapes(""), "");
    }

    #[test]
    fn parse_unknown_escape_keeps_backslash() {
        assert_eq!(parse_escapes(r"\x"), "\\x");
    }

    #[test]
    fn parse_cjk_preserved() {
        assert_eq!(parse_escapes("中文\\n日文"), "中文\n日文");
    }

    #[test]
    fn parse_emoji_preserved() {
        assert_eq!(parse_escapes("hello 🌍\\nworld"), "hello 🌍\nworld");
    }

    // ── truncate_display ──

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate_display("hello", 10), "hello");
    }

    #[test]
    fn truncate_exact_length_unchanged() {
        assert_eq!(truncate_display("hello", 5), "hello");
    }

    #[test]
    fn truncate_long_string_cut() {
        assert_eq!(truncate_display("hello world", 5), "hello...");
    }

    #[test]
    fn truncate_cjk_chars() {
        assert_eq!(truncate_display("中文字符测试", 3), "中文字...");
    }

    #[test]
    fn truncate_empty_string() {
        assert_eq!(truncate_display("", 5), "");
    }

    #[test]
    fn truncate_zero_max() {
        assert_eq!(truncate_display("hello", 0), "...");
    }

    #[test]
    fn truncate_emoji_boundary() {
        assert_eq!(truncate_display("a🌍b🌍c", 3), "a🌍b...");
    }
}
