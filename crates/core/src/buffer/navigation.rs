use std::ops::Range;

use unicode_categories::UnicodeCategories;

use crate::buffer::text_buffer::{CursorMovement, TextBuffer};
use crate::document::ReadableDocument;
use crate::helpers::CoordType;
use crate::simd;
use crate::types::{ByteIndex, LogicalPoint, VisualPoint};
use crate::unicode::{Cursor, CursorNav};

#[derive(Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Whitespace,
    Newline,
    Separator,
    Word,
}

const fn construct_classifier(separators: &[u8]) -> [CharClass; 256] {
    let mut classifier = [CharClass::Word; 256];

    classifier[b' ' as usize] = CharClass::Whitespace;
    classifier[b'\t' as usize] = CharClass::Whitespace;
    classifier[b'\n' as usize] = CharClass::Newline;
    classifier[b'\r' as usize] = CharClass::Newline;

    let mut i = 0;
    let len = separators.len();
    while i < len {
        let ch = separators[i];
        assert!(ch < 128, "Only ASCII separators are supported.");
        classifier[ch as usize] = CharClass::Separator;
        i += 1;
    }

    classifier
}

const WORD_CLASSIFIER: [CharClass; 256] =
    construct_classifier(br#"`~!@#$%^&*()-=+[{]}\|;:'",.<>/?"#);

/// Returns the byte length of a UTF-8 character from its first byte.
#[inline]
fn utf8_char_len(first: u8) -> usize {
    if first & 0x80 == 0 {
        1
    } else if first & 0xE0 == 0xC0 {
        2
    } else if first & 0xF0 == 0xE0 {
        3
    } else if first & 0xF8 == 0xF0 {
        4
    } else {
        1
    }
}

/// Unicode-aware character classification for word selection.
/// For ASCII bytes (< 128), uses the fast WORD_CLASSIFIER lookup table.
/// For non-ASCII bytes (>= 128), decodes the UTF-8 codepoint and checks
/// Unicode General Category to distinguish CJK punctuation from CJK words.
fn byte_class(bytes: &[u8], pos: usize) -> CharClass {
    let b = bytes[pos];
    if b < 128 {
        return WORD_CLASSIFIER[b as usize];
    }
    if b & 0xC0 == 0x80 {
        // Continuation byte — invalid first byte position
        return CharClass::Word;
    }
    let (codepoint, _) = if b & 0xE0 == 0xC0 {
        if pos + 1 >= bytes.len() {
            return CharClass::Word;
        }
        let cp = ((b as u32 & 0x1F) << 6) | (bytes[pos + 1] as u32 & 0x3F);
        (cp, 2)
    } else if b & 0xF0 == 0xE0 {
        if pos + 2 >= bytes.len() {
            return CharClass::Word;
        }
        let cp = ((b as u32 & 0x0F) << 12)
            | ((bytes[pos + 1] as u32 & 0x3F) << 6)
            | (bytes[pos + 2] as u32 & 0x3F);
        (cp, 3)
    } else if b & 0xF8 == 0xF0 {
        if pos + 3 >= bytes.len() {
            return CharClass::Word;
        }
        let cp = ((b as u32 & 0x07) << 18)
            | ((bytes[pos + 1] as u32 & 0x3F) << 12)
            | ((bytes[pos + 2] as u32 & 0x3F) << 6)
            | (bytes[pos + 3] as u32 & 0x3F);
        (cp, 4)
    } else {
        return CharClass::Word;
    };
    let ch = match char::from_u32(codepoint) {
        Some(c) => c,
        None => return CharClass::Word,
    };
    if ch.is_punctuation() || ch.is_symbol() { CharClass::Separator } else { CharClass::Word }
}

/// Finds the next word boundary given a document cursor offset.
/// Returns the offset of the next word boundary.
pub fn word_forward(doc: &dyn ReadableDocument, offset: usize) -> usize {
    word_navigation(WordForward { doc, offset, chunk: &[], chunk_off: 0 })
}

/// The backward version of `word_forward`.
pub fn word_backward(doc: &dyn ReadableDocument, offset: usize) -> usize {
    word_navigation(WordBackward { doc, offset, chunk: &[], chunk_off: 0 })
}

/// Word navigation implementation. Matches the behavior of VS Code.
fn word_navigation<T: WordNavigation>(mut nav: T) -> usize {
    // First, fill `self.chunk` with at least 1 grapheme.
    nav.read();

    // Skip one newline, if any.
    nav.skip_newline();

    // Skip any whitespace.
    nav.skip_class(CharClass::Whitespace);

    // Skip one word or separator and take note of the class.
    let class = nav.peek(CharClass::Whitespace);
    if matches!(class, CharClass::Separator | CharClass::Word) {
        nav.next();

        let off = nav.offset();

        // Continue skipping the same class.
        nav.skip_class(class);

        // If the class was a separator and we only moved one character,
        // continue skipping characters of the word class.
        if off == nav.offset() && class == CharClass::Separator {
            nav.skip_class(CharClass::Word);
        }
    }

    nav.offset()
}

trait WordNavigation {
    fn read(&mut self);
    fn skip_newline(&mut self);
    fn skip_class(&mut self, class: CharClass);
    fn peek(&self, default: CharClass) -> CharClass;
    fn next(&mut self);
    fn offset(&self) -> usize;
}

struct WordForward<'a> {
    doc: &'a dyn ReadableDocument,
    offset: usize,
    chunk: &'a [u8],
    chunk_off: usize,
}

impl WordNavigation for WordForward<'_> {
    fn read(&mut self) {
        self.chunk = self.doc.read_forward(self.offset);
        self.chunk_off = 0;
    }

    fn skip_newline(&mut self) {
        // We can rely on the fact that the document does not split graphemes across chunks.
        // = If there's a newline it's wholly contained in this chunk.
        // Unlike with `WordBackward`, we can't check for CR and LF separately as only a CR followed
        // by a LF is a newline. A lone CR in the document is just a regular control character.
        self.chunk_off += match self.chunk.get(self.chunk_off) {
            Some(&b'\n') => 1,
            Some(&b'\r') if self.chunk.get(self.chunk_off + 1) == Some(&b'\n') => 2,
            _ => 0,
        }
    }

    fn skip_class(&mut self, class: CharClass) {
        while !self.chunk.is_empty() {
            while self.chunk_off < self.chunk.len() {
                if byte_class(self.chunk, self.chunk_off) != class {
                    return;
                }
                self.chunk_off += utf8_char_len(self.chunk[self.chunk_off]);
            }

            self.offset += self.chunk.len();
            self.chunk = self.doc.read_forward(self.offset);
            self.chunk_off = 0;
        }
    }

    fn peek(&self, default: CharClass) -> CharClass {
        if self.chunk_off < self.chunk.len() {
            byte_class(self.chunk, self.chunk_off)
        } else {
            default
        }
    }

    fn next(&mut self) {
        self.chunk_off += 1;
    }

    fn offset(&self) -> usize {
        self.offset + self.chunk_off
    }
}

struct WordBackward<'a> {
    doc: &'a dyn ReadableDocument,
    offset: usize,
    chunk: &'a [u8],
    chunk_off: usize,
}

impl WordNavigation for WordBackward<'_> {
    fn read(&mut self) {
        self.chunk = self.doc.read_backward(self.offset);
        self.chunk_off = self.chunk.len();
    }

    fn skip_newline(&mut self) {
        // We can rely on the fact that the document does not split graphemes across chunks.
        // = If there's a newline it's wholly contained in this chunk.
        if self.chunk_off > 0 && self.chunk[self.chunk_off - 1] == b'\n' {
            self.chunk_off -= 1;
        }
        if self.chunk_off > 0 && self.chunk[self.chunk_off - 1] == b'\r' {
            self.chunk_off -= 1;
        }
    }

    fn skip_class(&mut self, class: CharClass) {
        while !self.chunk.is_empty() {
            while self.chunk_off > 0 {
                // Find the start of the previous UTF-8 character
                let mut start = self.chunk_off - 1;
                while start > 0 && (self.chunk[start] & 0xC0) == 0x80 {
                    start -= 1;
                }
                if byte_class(self.chunk, start) != class {
                    return;
                }
                self.chunk_off = start;
            }

            self.offset -= self.chunk.len();
            self.chunk = self.doc.read_backward(self.offset);
            self.chunk_off = self.chunk.len();
        }
    }

    fn peek(&self, default: CharClass) -> CharClass {
        if self.chunk_off > 0 {
            let mut start = self.chunk_off - 1;
            while start > 0 && (self.chunk[start] & 0xC0) == 0x80 {
                start -= 1;
            }
            byte_class(self.chunk, start)
        } else {
            default
        }
    }

    fn next(&mut self) {
        self.chunk_off -= 1;
    }

    fn offset(&self) -> usize {
        self.offset - self.chunk.len() + self.chunk_off
    }
}

/// Returns the offset range of the "word" at the given offset.
/// Does not cross newlines. Works similar to VS Code.
pub fn word_select(doc: &dyn ReadableDocument, offset: usize) -> Range<usize> {
    // Align offset to the start of the UTF-8 character.
    // If `offset` lands on a continuation byte, back up to the first byte.
    let mut offset = offset;
    {
        let probe = doc.read_forward(offset);
        if !probe.is_empty() && (probe[0] & 0xC0) == 0x80 {
            // Back up to find the first byte of this multi-byte char
            let back = doc.read_backward(offset);
            for i in (0..back.len()).rev() {
                if (back[i] & 0xC0) != 0x80 {
                    offset -= back.len() - i;
                    break;
                }
            }
        }
    }

    let mut beg = offset;
    let mut end = offset;
    let mut class = CharClass::Newline;

    let mut chunk = doc.read_forward(end);
    if !chunk.is_empty() {
        // Not at the end of the document? Great!
        // We default to using the next char as the class, because in terminals
        // the cursor is usually always to the left of the cell you clicked on.
        class = byte_class(chunk, 0);

        let mut chunk_off = 0;

        // Select the word, unless we hit a newline.
        if class != CharClass::Newline {
            loop {
                let char_len = utf8_char_len(chunk[chunk_off]);
                chunk_off += char_len;
                end += char_len;

                if chunk_off >= chunk.len() {
                    chunk = doc.read_forward(end);
                    chunk_off = 0;
                    if chunk.is_empty() {
                        break;
                    }
                }

                if byte_class(chunk, chunk_off) != class {
                    break;
                }
            }
        }
    }

    let mut chunk = doc.read_backward(beg);
    if !chunk.is_empty() {
        let mut chunk_off = chunk.len();

        // If we failed to determine the class, because we hit the end of the document
        // or a newline, we fall back to using the previous character, of course.
        if class == CharClass::Newline {
            let mut prev_start = chunk_off - 1;
            while prev_start > 0 && (chunk[prev_start] & 0xC0) == 0x80 {
                prev_start -= 1;
            }
            class = byte_class(chunk, prev_start);
        }

        // Select the word, unless we hit a newline.
        if class != CharClass::Newline {
            loop {
                // Find the start of the previous UTF-8 character
                let mut start = chunk_off - 1;
                while start > 0 && (chunk[start] & 0xC0) == 0x80 {
                    start -= 1;
                }
                if byte_class(chunk, start) != class {
                    break;
                }

                let char_len = chunk_off - start;
                chunk_off -= char_len;
                beg -= char_len;

                if chunk_off == 0 {
                    chunk = doc.read_backward(beg);
                    chunk_off = chunk.len();
                    if chunk.is_empty() {
                        break;
                    }
                }
            }
        }
    }

    beg..end
}

impl TextBuffer {
    fn cursor_nav(&self) -> CursorNav<'_> {
        CursorNav::new(&self.buffer).with_tab_size(self.tab_size)
    }

    pub(crate) fn goto_line_start(&self, cursor: Cursor, target_y: usize) -> Cursor {
        let mut result = cursor;
        let mut seek_to_line_start = true;

        if target_y > result.logical_pos.line {
            while target_y > result.logical_pos.line {
                let chunk = self.read_forward(result.offset.to_usize());
                if chunk.is_empty() {
                    break;
                }

                let (delta, line) = simd::lines_fwd(
                    chunk,
                    0,
                    result.logical_pos.line as CoordType,
                    target_y as CoordType,
                );
                result.offset = ByteIndex(result.offset.to_usize() + delta);
                result.logical_pos.line = line.max(0) as usize;
            }

            // If we're at the end of the buffer, we could either be there because the last
            // character in the buffer is genuinely a newline, or because the buffer ends in a
            // line of text without trailing newline. The only way to make sure is to seek
            // backwards to the line start again. But otherwise we can skip that.
            seek_to_line_start =
                result.offset.to_usize() == self.text_length() && result.offset != cursor.offset;
        }

        if seek_to_line_start {
            loop {
                let chunk = self.read_backward(result.offset.to_usize());
                if chunk.is_empty() {
                    break;
                }

                let (delta, line) = simd::lines_bwd(
                    chunk,
                    chunk.len(),
                    result.logical_pos.line as CoordType,
                    target_y as CoordType,
                );
                result.offset = ByteIndex(result.offset.to_usize() - (chunk.len() - delta));
                result.logical_pos.line = line.max(0) as usize;
                if delta > 0 {
                    break;
                }
            }
        }

        if result.offset == cursor.offset {
            return result;
        }

        result.logical_pos.unichar = 0;
        result.visual_pos.column = 0;
        result.visual_pos.row = result.logical_pos.line;
        result.column = 0;

        result
    }

    pub(crate) fn cursor_move_to_byte_internal(
        &self,
        mut cursor: Cursor,
        offset: ByteIndex,
    ) -> Cursor {
        if offset == cursor.offset {
            return cursor;
        }

        // goto_line_start() is fast for seeking across lines _if_ line wrapping is disabled.
        // For backward seeking we have to use it either way, so we're covered there.
        // This implements the forward seeking portion, if it's approx. worth doing so.
        if offset.to_usize().saturating_sub(cursor.offset.to_usize()) > 1024 {
            // Replacing this with a more optimal, direct memchr() loop appears
            // to improve performance only marginally by another 2% or so.
            // Still, it's kind of "meh" looking at how poorly this is implemented...
            loop {
                let next = self.goto_line_start(cursor, cursor.logical_pos.line + 1);
                // Stop when we either ran past the target offset,
                // or when we hit the end of the buffer and `goto_line_start` backtracked to the line start.
                if next.offset > offset || next.offset <= cursor.offset {
                    break;
                }
                cursor = next;
            }
        }

        while offset.to_usize() < cursor.offset.to_usize() {
            // At line 0, seek to the line start so goto_byte() can navigate forward.
            if cursor.logical_pos.line == 0 {
                cursor = self.goto_line_start(cursor, 0);
                break;
            }
            cursor = self.goto_line_start(cursor, cursor.logical_pos.line - 1);
        }

        self.cursor_nav().with_cursor(cursor).goto_byte(offset)
    }

    pub(crate) fn cursor_move_to_logical_internal(
        &self,
        mut cursor: Cursor,
        pos: LogicalPoint,
    ) -> Cursor {
        if pos == cursor.logical_pos {
            return cursor;
        }

        // goto_line_start() is the fastest way for seeking across lines. As such we always
        // use it if the requested `.line` position is different. We still need to use it if
        // the `.unichar` position is smaller, but only because `goto_logical()` cannot seek backwards.
        if pos.line != cursor.logical_pos.line || pos.unichar < cursor.logical_pos.unichar {
            cursor = self.goto_line_start(cursor, pos.line);
        }

        self.cursor_nav().with_cursor(cursor).goto_logical(pos)
    }

    pub(crate) fn cursor_move_to_visual_internal(
        &self,
        mut cursor: Cursor,
        pos: VisualPoint,
    ) -> Cursor {
        if pos == cursor.visual_pos {
            return cursor;
        }

        // word_wrap always disabled: visual == logical.
        if pos.row != cursor.visual_pos.row || pos.column < cursor.visual_pos.column {
            cursor = self.goto_line_start(cursor, pos.row);
        }

        self.cursor_nav().with_cursor(cursor).goto_visual(pos)
    }

    pub(crate) fn cursor_move_delta_internal(
        &self,
        mut cursor: Cursor,
        granularity: CursorMovement,
        mut delta: isize,
    ) -> Cursor {
        if delta == 0 {
            return cursor;
        }

        let sign = if delta > 0 { 1 } else { -1 };

        match granularity {
            CursorMovement::Grapheme => {
                let start_unichar: usize = if delta > 0 { 0 } else { usize::MAX };

                loop {
                    let target_unichar_signed = cursor.logical_pos.unichar as isize + delta;
                    // Clamp to 0 for the seek (negative means "before this line"),
                    // but keep the signed value for the remaining-delta calculation
                    // so the cross-line logic below triggers correctly.
                    let target_unichar_clamped = target_unichar_signed.max(0) as usize;

                    cursor = self.cursor_move_to_logical_internal(
                        cursor,
                        LogicalPoint {
                            unichar: target_unichar_clamped,
                            line: cursor.logical_pos.line,
                        },
                    );

                    // We can stop if we ran out of remaining delta
                    // (or perhaps ran past the goal; in either case the sign would've changed),
                    // or if we hit the beginning or end of the buffer.
                    let remaining = target_unichar_signed - cursor.logical_pos.unichar as isize;
                    if remaining.signum() != sign
                        || (remaining < 0 && cursor.offset == ByteIndex::ZERO)
                        || (remaining > 0 && cursor.offset.to_usize() >= self.text_length())
                    {
                        break;
                    }

                    let target_line = (cursor.logical_pos.line as isize + sign).max(0) as usize;
                    cursor = self.cursor_move_to_logical_internal(
                        cursor,
                        LogicalPoint { unichar: start_unichar, line: target_line },
                    );

                    // We crossed a newline which counts for 1 grapheme cluster.
                    // So, we also need to run the same check again.
                    delta = remaining - sign;
                    if delta.signum() != sign
                        || cursor.offset == ByteIndex::ZERO
                        || cursor.offset.to_usize() >= self.text_length()
                    {
                        break;
                    }
                }
            }
            CursorMovement::Word => {
                let doc = &self.buffer as &dyn ReadableDocument;
                let mut offset = cursor.offset.to_usize();

                while delta != 0 {
                    if delta < 0 {
                        offset = super::navigation::word_backward(doc, offset);
                    } else {
                        offset = super::navigation::word_forward(doc, offset);
                    }
                    delta -= sign;
                }

                cursor = self.cursor_move_to_byte_internal(cursor, ByteIndex(offset));
            }
        }

        cursor
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_word_navigation() {
        assert_eq!(word_forward(&"Hello World".as_bytes(), 0), 5);
        assert_eq!(word_forward(&"Hello,World".as_bytes(), 0), 5);
        assert_eq!(word_forward(&"   Hello".as_bytes(), 0), 8);
        assert_eq!(word_forward(&"\n\nHello".as_bytes(), 0), 1);

        assert_eq!(word_backward(&"Hello World".as_bytes(), 11), 6);
        assert_eq!(word_backward(&"Hello,World".as_bytes(), 10), 6);
        assert_eq!(word_backward(&"Hello   ".as_bytes(), 7), 0);
        assert_eq!(word_backward(&"Hello\n\n".as_bytes(), 7), 6);
    }

    #[test]
    fn word_select_cjk_punctuation_boundary() {
        // Chinese comma should break word selection
        let doc = "你好，世界".as_bytes();
        // Click on 你 — should select only 你好 (not 你好，世界)
        let r = word_select(&doc, 0);
        assert_eq!(r, 0..6, "should select 你好 (6 bytes)");
        // Click on 世 — should select only 世界
        let r = word_select(&doc, 9);
        assert_eq!(r, 9..15, "should select 世界 (6 bytes)");
        // Click on ， — it is Separator class, selects just the comma
        let r = word_select(&doc, 6);
        assert_eq!(r, 6..9, "should select ， (3 bytes)");
    }

    #[test]
    fn word_select_cjk_period_boundary() {
        let doc = "测试。完成".as_bytes();
        let r = word_select(&doc, 0);
        assert_eq!(r, 0..6, "should select 测试");
        let r = word_select(&doc, 9);
        assert_eq!(r, 9..15, "should select 完成");
    }

    #[test]
    fn word_select_cjk_mixed_punctuation() {
        // Mix of Chinese and ASCII punctuation
        let doc = "hello，世界！".as_bytes();
        let r = word_select(&doc, 0);
        assert_eq!(r, 0..5, "should select hello");
        let r = word_select(&doc, 7);
        assert_eq!(r, 5..8, "continuation byte in ， selects the comma");
    }

    #[test]
    fn word_select_japanese_punctuation() {
        // Japanese also uses CJK punctuation
        let doc = "テスト、完了".as_bytes();
        let r = word_select(&doc, 0);
        assert_eq!(r, 0..9, "should select テスト (3 katakana chars)");
        let r = word_select(&doc, 15);
        assert_eq!(r, 12..18, "continuation byte in 、 selects the comma");
    }
}
