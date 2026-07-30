//! Abstractions over reading/writing arbitrary text containers.

use std::ffi::OsString;
use std::mem;
use std::ops::Range;
use std::path::PathBuf;

use stdext::ReplaceRange as _;

/// An abstraction over reading from text containers.
pub trait ReadableDocument {
    /// Read some bytes starting at (including) the given absolute offset.
    ///
    /// # Warning
    ///
    /// * Be lenient on inputs:
    ///   * The given offset may be out of bounds and you MUST clamp it.
    ///   * You should not assume that offsets are at grapheme cluster boundaries.
    /// * Be strict on outputs:
    ///   * You MUST NOT break grapheme clusters across chunks.
    ///   * You MUST NOT return an empty slice unless the offset is at or beyond the end.
    fn read_forward(&self, off: usize) -> &[u8];

    /// Read some bytes before (but not including) the given absolute offset.
    ///
    /// # Warning
    ///
    /// * Be lenient on inputs:
    ///   * The given offset may be out of bounds and you MUST clamp it.
    ///   * You should not assume that offsets are at grapheme cluster boundaries.
    /// * Be strict on outputs:
    ///   * You MUST NOT break grapheme clusters across chunks.
    ///   * You MUST NOT return an empty slice unless the offset is zero.
    fn read_backward(&self, off: usize) -> &[u8];
}

/// An abstraction over writing to text containers.
pub trait WriteableDocument: ReadableDocument {
    /// Replace the given range with the given bytes.
    ///
    /// # Warning
    ///
    /// * The given range may be out of bounds and you MUST clamp it.
    /// * The replacement may not be valid UTF8.
    fn replace(&mut self, range: Range<usize>, replacement: &[u8]);
}

impl ReadableDocument for &[u8] {
    fn read_forward(&self, off: usize) -> &[u8] {
        let s = *self;
        &s[off.min(s.len())..]
    }

    fn read_backward(&self, off: usize) -> &[u8] {
        let s = *self;
        &s[..off.min(s.len())]
    }
}

impl ReadableDocument for String {
    fn read_forward(&self, off: usize) -> &[u8] {
        let s = self.as_bytes();
        &s[off.min(s.len())..]
    }

    fn read_backward(&self, off: usize) -> &[u8] {
        let s = self.as_bytes();
        &s[..off.min(s.len())]
    }
}

impl WriteableDocument for String {
    fn replace(&mut self, range: Range<usize>, replacement: &[u8]) {
        // `replacement` is not guaranteed to be valid UTF-8, so we need to sanitize it.
        let utf8 = String::from_utf8_lossy(replacement);
        // SAFETY: `range` is guaranteed to be on codepoint boundaries.
        unsafe { self.as_mut_vec() }.replace_range(range, utf8.as_bytes());
    }
}

impl ReadableDocument for PathBuf {
    fn read_forward(&self, off: usize) -> &[u8] {
        let s = self.as_os_str().as_encoded_bytes();
        &s[off.min(s.len())..]
    }

    fn read_backward(&self, off: usize) -> &[u8] {
        let s = self.as_os_str().as_encoded_bytes();
        &s[..off.min(s.len())]
    }
}

impl WriteableDocument for PathBuf {
    fn replace(&mut self, range: Range<usize>, replacement: &[u8]) {
        let mut vec = mem::take(self).into_os_string().into_encoded_bytes();
        vec.replace_range(range, replacement);
        *self = unsafe { Self::from(OsString::from_encoded_bytes_unchecked(vec)) };
    }
}

/// Read-only snapshot of a document viewport for plugins / UI layers.
pub trait DocView {
    /// Total number of lines in the document.
    fn line_count(&self) -> usize;

    /// Raw text of the given `line`.
    fn doc_line_text(&self, line: usize) -> std::borrow::Cow<'_, str>;

    /// Text within the given byte range.
    fn doc_text_in_range(&self, range: std::ops::Range<usize>) -> std::borrow::Cow<'_, str>;

    /// Absolute byte offset where `line` starts within the document.
    fn line_byte_offset(&self, line: usize) -> usize;

    /// Byte length of `line` (excluding any trailing newline).
    fn line_byte_length(&self, line: usize) -> usize;

    /// Current vertical scroll position in pixels.
    fn scroll_y(&self) -> f32;

    /// Visible viewport height in pixels.
    fn viewport_height(&self) -> f32;

    /// Convenience: `true` when the document contains no lines.
    fn is_empty(&self) -> bool {
        self.line_count() == 0
    }
}

/// Mutable viewport access — extends [`DocView`] with scroll control.
pub trait DocViewMut: DocView {
    /// Set the vertical scroll position in pixels.
    fn set_scroll_y(&mut self, y: f32);

    /// 替换 [range] 字节区间的文本为 text。
    fn replace_range(&mut self, range: std::ops::Range<usize>, text: &str);

    /// 开始一个编辑事务——后续多次 replace_range 合并为一个 Undo 单元。
    fn begin_edit(&mut self) {}

    /// 结束编辑事务。
    fn end_edit(&mut self) {}
}

/// A simple read-only DocView backed by a string.
pub struct StringDocView<'a> {
    src: &'a str,
    line_offsets: Vec<usize>,
}

impl<'a> StringDocView<'a> {
    pub fn new(src: &'a str) -> Self {
        let mut line_offsets = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_offsets.push(i + 1);
            }
        }
        Self { src, line_offsets }
    }
}

impl<'a> DocView for StringDocView<'a> {
    fn line_count(&self) -> usize {
        self.line_offsets.len()
    }
    fn doc_line_text(&self, line: usize) -> std::borrow::Cow<'_, str> {
        let start = self.line_offsets[line];
        let end =
            self.line_offsets.get(line + 1).map(|&e| e.saturating_sub(1)).unwrap_or(self.src.len());
        std::borrow::Cow::Borrowed(&self.src[start..end])
    }
    fn doc_text_in_range(&self, range: std::ops::Range<usize>) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(
            &self.src[range.start.min(self.src.len())..range.end.min(self.src.len())],
        )
    }
    fn line_byte_offset(&self, line: usize) -> usize {
        self.line_offsets[line]
    }
    fn line_byte_length(&self, line: usize) -> usize {
        let start = self.line_offsets[line];
        let end =
            self.line_offsets.get(line + 1).map(|&e| e.saturating_sub(1)).unwrap_or(self.src.len());
        end - start
    }
    fn scroll_y(&self) -> f32 {
        0.0
    }
    fn viewport_height(&self) -> f32 {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // --- String tests ---

    #[test]
    fn string_read_forward_empty() {
        let s = String::new();
        assert_eq!(s.read_forward(0), b"");
    }

    #[test]
    fn string_read_forward_basic() {
        let s = "hello".to_string();
        assert_eq!(s.read_forward(0), b"hello");
        assert_eq!(s.read_forward(3), b"lo");
        assert_eq!(s.read_forward(5), b"");
    }

    #[test]
    fn string_read_forward_clamped() {
        let s = "hi".to_string();
        assert_eq!(s.read_forward(100), b"");
    }

    #[test]
    fn string_read_backward_empty() {
        let s = String::new();
        assert_eq!(s.read_backward(0), b"");
    }

    #[test]
    fn string_read_backward_basic() {
        let s = "hello".to_string();
        assert_eq!(s.read_backward(5), b"hello");
        assert_eq!(s.read_backward(3), b"hel");
        assert_eq!(s.read_backward(0), b"");
    }

    #[test]
    fn string_read_backward_clamped() {
        let s = "hi".to_string();
        assert_eq!(s.read_backward(100), b"hi");
    }

    #[test]
    fn string_replace_basic() {
        let mut s = "hello".to_string();
        s.replace(1..4, b"OO");
        assert_eq!(s, "hOOo");
    }

    #[test]
    fn string_replace_empty() {
        let mut s = "hello".to_string();
        s.replace(0..0, b"X");
        assert_eq!(s, "Xhello");
    }

    #[test]
    fn string_replace_lossy() {
        let mut s = "hello".to_string();
        // replacement with invalid UTF-8 should be lossy
        s.replace(0..5, &[0xFF, 0xFE]);
        assert!(s.contains('\u{FFFD}'));
    }

    // --- PathBuf tests ---

    #[test]
    fn pathbuf_read_forward_basic() {
        let p = PathBuf::from("/a/b");
        let bytes = p.read_forward(0);
        assert_eq!(bytes, b"/a/b");
    }

    #[test]
    fn pathbuf_read_backward_basic() {
        let p = PathBuf::from("/a/b");
        let bytes = p.read_backward(3);
        assert_eq!(bytes, b"/a/");
    }

    #[test]
    fn pathbuf_replace_basic() {
        let mut p = PathBuf::from("/a/b/c");
        p.replace(3..4, b"xyz");
        assert_eq!(p.to_str().unwrap(), "/a/xyz/c");
    }
}
