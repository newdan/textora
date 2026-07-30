//! SIMD-accelerated search — direct byte-level search without regex/ICU.
//!
//! Phase 10 non-regex search path. Uses `memchr2` for fast
//! single-byte scanning and SIMD-widened substring verification.

use std::ops::Range;

use crate::simd::memchr2;

/// Find all non-overlapping occurrences of a single ASCII byte.
pub fn find_all_ascii(needle: u8, haystack: &[u8]) -> Vec<Range<usize>> {
    let mut matches = Vec::new();
    let mut off = 0usize;
    loop {
        off = memchr2(needle, needle, haystack, off);
        if off >= haystack.len() {
            break;
        }
        matches.push(off..off + 1);
        off += 1;
    }
    matches
}

/// Find all non-overlapping occurrences of a substring.
pub fn find_all(needle: &[u8], haystack: &[u8]) -> Vec<Range<usize>> {
    if needle.is_empty() || haystack.is_empty() {
        return Vec::new();
    }
    let first_byte = needle[0];
    let needle_len = needle.len();
    let mut matches = Vec::new();
    let mut off = 0usize;

    loop {
        off = memchr2(first_byte, first_byte, haystack, off);
        if off + needle_len > haystack.len() {
            break;
        }
        if &haystack[off..off + needle_len] == needle {
            matches.push(off..off + needle_len);
            off += needle_len;
        } else {
            off += 1;
        }
    }
    matches
}

/// Find all overlapping occurrences.
pub fn find_all_overlapping(needle: &[u8], haystack: &[u8]) -> Vec<Range<usize>> {
    if needle.is_empty() || haystack.is_empty() {
        return Vec::new();
    }
    let first_byte = needle[0];
    let needle_len = needle.len();
    let mut matches = Vec::new();
    let mut off = 0usize;

    loop {
        off = memchr2(first_byte, first_byte, haystack, off);
        if off + needle_len > haystack.len() {
            break;
        }
        if &haystack[off..off + needle_len] == needle {
            matches.push(off..off + needle_len);
        }
        off += 1;
    }
    matches
}

/// Case-insensitive ASCII search.
pub fn find_all_case_insensitive_ascii(needle: &[u8], haystack: &[u8]) -> Vec<Range<usize>> {
    if needle.is_empty() || haystack.is_empty() {
        return Vec::new();
    }
    let needle_len = needle.len();
    let needle_lower: Vec<u8> =
        needle.iter().map(|&b| if b.is_ascii_uppercase() { b + 32 } else { b }).collect();

    let first_byte = needle_lower[0];
    let first_upper = if first_byte.is_ascii_lowercase() { first_byte - 32 } else { first_byte };

    let mut matches = Vec::new();
    let mut off = 0usize;

    loop {
        off = memchr2(first_byte, first_upper, haystack, off);
        if off + needle_len > haystack.len() {
            break;
        }
        let candidate = &haystack[off..off + needle_len];
        let match_found = candidate.iter().zip(needle_lower.iter()).all(|(&a, &b)| {
            let a_lower = if a.is_ascii_uppercase() { a + 32 } else { a };
            a_lower == b
        });
        if match_found {
            matches.push(off..off + needle_len);
            off += needle_len;
        } else {
            off += 1;
        }
    }
    matches
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_find_empty() {
        assert!(find_all(b"", b"abc").is_empty());
    }

    #[test]
    fn test_find_basic() {
        let h = b"the quick brown fox";
        assert_eq!(find_all(b"quick", h), vec![4..9]);
    }

    #[test]
    fn test_find_overlapping() {
        assert_eq!(find_all_overlapping(b"aa", b"aaaa"), vec![0..2, 1..3, 2..4]);
    }

    #[test]
    fn test_find_case_insensitive() {
        let h = b"Hello HELLO hello";
        assert_eq!(find_all_case_insensitive_ascii(b"hello", h).len(), 3);
    }

    #[test]
    fn test_find_cjk() {
        let h = "你好世界你好".as_bytes();
        assert_eq!(find_all("世界".as_bytes(), h), vec![6..12]);
    }
}
