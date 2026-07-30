//! Search and replace functionality for TextBuffer.

use std::borrow::Cow;
use std::cell::UnsafeCell;
use std::ops::Range;

use stdext::arena::{Arena, scratch_arena};
use stdext::collections::BVec;

use crate::buffer::history::TextBufferSelection;
use crate::buffer::text_buffer::{CursorMovement, TextBuffer};
use crate::icu;
use crate::simd::memchr2;
use crate::types::ByteIndex;

/// Options for a search operation.
#[derive(Default, Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchOptions {
    /// If true, the search is case-sensitive.
    pub match_case: bool,
    /// If true, the search matches whole words.
    pub whole_word: bool,
    /// If true, the search uses regex.
    pub use_regex: bool,
}

/// Caches an ICU search operation.
pub(crate) struct ActiveSearch {
    /// The search pattern.
    pub pattern: String,
    /// The search options.
    pub options: SearchOptions,
    /// The ICU `UText` object.
    pub text: icu::Text,
    /// The ICU `URegularExpression` object.
    pub regex: icu::Regex,
    /// `GapBuffer::generation` when the search was created.
    pub buffer_generation: u32,
    /// `TextBuffer::selection_generation` when the search was created.
    pub selection_generation: u32,
    /// Stores the text buffer offset in between searches.
    pub next_search_offset: usize,
    /// If we know there were no hits, we can skip searching.
    pub no_matches: bool,
}

pub(crate) enum RegexReplacement<'a> {
    Group(i32),
    Text(BVec<'a, u8>),
}

impl TextBuffer {
    pub fn find_and_select(&mut self, pattern: &str, options: SearchOptions) -> icu::Result<()> {
        if let Some(search) = &mut self.search {
            let search = search.get_mut();
            // When the search input changes we must reset the search.
            if search.pattern != pattern || search.options != options {
                self.search = None;
            }

            // When transitioning from some search to no search, we must clear the selection.
            if pattern.is_empty()
                && let Some(TextBufferSelection { beg, .. }) = self.selection
            {
                self.cursor_move_to_logical(beg);
            }
        }

        if pattern.is_empty() {
            return Ok(());
        }

        let search = match &self.search {
            Some(search) => unsafe { &mut *search.get() },
            None => {
                let search = self.find_construct_search(pattern, options)?;
                self.search = Some(UnsafeCell::new(search));
                unsafe { &mut *self.search.as_ref().unwrap().get() }
            }
        };

        // If we previously searched through the entire document and found 0 matches,
        // then we can avoid searching again.
        if search.no_matches {
            return Ok(());
        }

        // If the user moved the cursor since the last search, but the needle remained the same,
        // we still need to move the start of the search to the new cursor position.
        let next_search_offset = if self.selection_generation == search.selection_generation {
            search.next_search_offset
        } else {
            match self.selection {
                Some(TextBufferSelection { beg, end }) => self
                    .cursor_move_to_logical_internal(self.cursor, beg.min(end))
                    .offset
                    .to_usize(),
                _ => self.cursor.offset.to_usize(),
            }
        };

        self.find_select_next(search, next_search_offset, true)?;
        Ok(())
    }

    /// Find the next occurrence of the given `pattern` and replace it with `replacement`.
    pub fn find_and_replace(
        &mut self,
        pattern: &str,
        options: SearchOptions,
        replacement: &[u8],
    ) -> icu::Result<()> {
        // Editors traditionally replace the previous search hit, not the next possible one.
        if let Some(search) = &self.search {
            let search = unsafe { &mut *search.get() };
            if search.selection_generation == self.selection_generation {
                let scratch = scratch_arena(None);
                let zero_width = self.selection.is_none();
                let parsed_replacements =
                    Self::find_parse_replacement(&scratch, &mut *search, replacement)?;
                let replacement =
                    self.find_fill_replacement(&mut *search, replacement, &parsed_replacements)?;
                self.write_raw(&replacement);

                // After replacing a zero-width match, advance past it so that find_and_select wraps to the
                // next match rather than finding the same anchor (e.g. `$`) again at the same line end.
                if zero_width {
                    search.next_search_offset =
                        self.find_advance_past_zero_width(self.active_edit_off).unwrap_or(0);
                }
            }
        }

        self.find_and_select(pattern, options)
    }

    /// Find all occurrences of the given `pattern` and replace them with `replacement`.
    pub fn find_and_replace_all(
        &mut self,
        pattern: &str,
        options: SearchOptions,
        replacement: &[u8],
    ) -> icu::Result<()> {
        self.edit_begin_grouping();
        let result = (|| {
            let scratch = scratch_arena(None);
            let mut search = self.find_construct_search(pattern, options)?;
            let mut offset = 0;
            let parsed_replacements =
                Self::find_parse_replacement(&scratch, &mut search, replacement)?;

            while let Some(range) = self.find_select_next(&mut search, offset, false)? {
                let replacement =
                    self.find_fill_replacement(&mut search, replacement, &parsed_replacements)?;
                self.write_raw(&replacement);

                // The `active_edit_off` points to the end of the last edit made by `write_raw()`.
                // This differs from the self.cursor.offset, if `write_raw()` did an `insert_final_newline`.
                offset = self.active_edit_off;

                // Avoid infinite loops when hitting zero-length matches
                // by advancing past the zero-length match location.
                //
                // This is technically not entirely correct. For instance imagine replacing
                // "^|f" with "x" in "foo". It should technically produce "xxoo", but I
                // found that other editors also do it wrong, so it can't matter too much.
                if range.is_empty() {
                    offset = match self.find_advance_past_zero_width(offset) {
                        Some(next) => next,
                        None => break,
                    };
                }
            }

            Ok(())
        })();
        self.edit_end_grouping();
        result
    }

    /// After replacing a zero-width match, compute the offset to resume
    /// searching from. Returns `None` if we're at the end of the buffer.
    fn find_advance_past_zero_width(&self, offset: usize) -> Option<usize> {
        let cursor = self.cursor_move_to_byte_internal(self.cursor, ByteIndex(offset));
        let next = self.cursor_move_delta_internal(cursor, CursorMovement::Grapheme, 1);
        (next.offset.to_usize() > offset).then_some(next.offset.to_usize())
    }

    fn find_construct_search(
        &self,
        pattern: &str,
        options: SearchOptions,
    ) -> icu::Result<ActiveSearch> {
        if pattern.is_empty() {
            return Err(icu::ILLEGAL_ARGUMENT_ERROR);
        }

        let sanitized_pattern = if options.whole_word && options.use_regex {
            Cow::Owned(format!(r"\b(?:{pattern})\b"))
        } else if options.whole_word {
            let mut p = String::with_capacity(pattern.len() + 16);
            p.push_str(r"\b");

            // Escape regex special characters.
            let b = unsafe { p.as_mut_vec() };
            for &byte in pattern.as_bytes() {
                match byte {
                    b'*' | b'?' | b'+' | b'[' | b'(' | b')' | b'{' | b'}' | b'^' | b'$' | b'|'
                    | b'\\' | b'.' => {
                        b.push(b'\\');
                        b.push(byte);
                    }
                    _ => b.push(byte),
                }
            }

            p.push_str(r"\b");
            Cow::Owned(p)
        } else {
            Cow::Borrowed(pattern)
        };

        let mut flags = icu::Regex::MULTILINE;
        if !options.match_case {
            flags |= icu::Regex::CASE_INSENSITIVE;
        }
        if !options.use_regex && !options.whole_word {
            flags |= icu::Regex::LITERAL;
        }

        // Move the start of the search to the start of the selection,
        // or otherwise to the current cursor position.

        let doc_len = self.buffer.len();
        let mut doc_bytes = Vec::with_capacity(doc_len);
        self.buffer.extract_raw(0..doc_len, &mut doc_bytes, 0);
        let text = unsafe { icu::Text::new(&doc_bytes)? };
        let regex = unsafe { icu::Regex::new(&sanitized_pattern, flags, &text)? };

        Ok(ActiveSearch {
            pattern: pattern.to_string(),
            options,
            text,
            regex,
            buffer_generation: self.buffer.generation(),
            selection_generation: 0,
            next_search_offset: 0,
            no_matches: false,
        })
    }

    fn find_select_next(
        &mut self,
        search: &mut ActiveSearch,
        offset: usize,
        wrap: bool,
    ) -> icu::Result<Option<Range<usize>>> {
        if search.buffer_generation != self.buffer.generation() {
            let doc_len = self.buffer.len();
            let mut doc_bytes = Vec::with_capacity(doc_len);
            self.buffer.extract_raw(0..doc_len, &mut doc_bytes, 0);
            search.text.rebuild(&doc_bytes)?;
            unsafe { search.regex.set_text(&search.text, offset)? };
            search.buffer_generation = self.buffer.generation();
            search.next_search_offset = offset;
        } else if search.next_search_offset != offset {
            search.next_search_offset = offset;
            search.regex.reset(offset)?;
        }

        let mut hit = search.regex.find_next()?;

        // If we hit the end of the buffer, and we know that there's something to find,
        // start the search again from the beginning (= wrap around).
        if wrap && hit.is_none() && search.next_search_offset != 0 {
            search.next_search_offset = 0;
            search.regex.reset(0)?;
            hit = search.regex.find_next()?;
        }

        search.selection_generation = if let Some(range) = &hit {
            // Now the search offset is no more at the start of the buffer.
            search.next_search_offset = range.end;

            let beg = self.cursor_move_to_byte_internal(self.cursor, ByteIndex(range.start));
            let end = self.cursor_move_to_byte_internal(beg, ByteIndex(range.end));

            unsafe { self.set_cursor(end) };
            self.make_cursor_visible();

            self.set_selection(Some(TextBufferSelection {
                beg: beg.logical_pos,
                end: end.logical_pos,
            }))
        } else {
            // Avoid searching through the entire document again if we know there's nothing to find.
            search.no_matches = true;
            self.set_selection(None)
        };

        Ok(hit)
    }

    fn find_parse_replacement<'a>(
        arena: &'a Arena,
        search: &mut ActiveSearch,
        replacement: &[u8],
    ) -> icu::Result<BVec<'a, RegexReplacement<'a>>> {
        let mut res = BVec::empty();

        if !search.options.use_regex {
            return Ok(res);
        }

        let group_count = search.regex.group_count()?;
        let mut text = BVec::empty();
        let mut text_beg = 0;

        loop {
            let mut off = memchr2(b'$', b'\\', replacement, text_beg);

            // Push the raw, unescaped text, if any.
            if text_beg < off {
                text.extend_from_slice(arena, &replacement[text_beg..off]);
            }

            // Unescape any escaped characters.
            while off < replacement.len() && replacement[off] == b'\\' {
                off += 2;

                // If this backslash is the last character (e.g. because
                // `replacement` is just 1 byte long, holding just b"\\"),
                // we can't unescape it. In that case, we map it to `b'\\'` here.
                // This results in us appending a literal backslash to the text.
                let ch = replacement.get(off - 1).map_or(b'\\', |&c| c);

                // Unescape and append the character.
                text.push(
                    arena,
                    match ch {
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        ch => ch,
                    },
                );
            }

            // Parse out a group number, if any.
            let mut group = -1;
            if off < replacement.len() && replacement[off] == b'$' {
                let mut beg = off;
                let mut end = off + 1;
                let mut acc = 0i32;
                let mut acc_bad = true;

                if end < replacement.len() {
                    let ch = replacement[end];

                    if ch == b'$' {
                        // Translate "$$" to "$".
                        beg += 1;
                        end += 1;
                    } else if ch.is_ascii_digit() {
                        // Parse "$1234" into 1234i32.
                        // If the number is larger than the group count,
                        // we flag `acc_bad` which causes us to treat it as text.
                        acc_bad = false;
                        while {
                            acc =
                                acc.wrapping_mul(10).wrapping_add((replacement[end] - b'0') as i32);
                            acc_bad |= acc > group_count;
                            end += 1;
                            end < replacement.len() && replacement[end].is_ascii_digit()
                        } {}
                    }
                }

                if !acc_bad {
                    group = acc;
                } else {
                    text.extend_from_slice(arena, &replacement[beg..end]);
                }

                off = end;
            }

            if !text.is_empty() {
                res.push(arena, RegexReplacement::Text(text));
                text = BVec::empty();
            }
            if group >= 0 {
                res.push(arena, RegexReplacement::Group(group));
            }

            text_beg = off;
            if text_beg >= replacement.len() {
                break;
            }
        }

        Ok(res)
    }

    fn find_fill_replacement<'a>(
        &self,
        search: &mut ActiveSearch,
        replacement: &'a [u8],
        parsed_replacements: &[RegexReplacement],
    ) -> icu::Result<Cow<'a, [u8]>> {
        if !search.options.use_regex {
            Ok(Cow::Borrowed(replacement))
        } else {
            let mut res = Vec::new();

            for replacement in parsed_replacements {
                match replacement {
                    RegexReplacement::Text(text) => res.extend_from_slice(text),
                    RegexReplacement::Group(group) => {
                        if let Some(range) = search.regex.group(*group)? {
                            self.buffer.extract_raw(range, &mut res, usize::MAX);
                        }
                    }
                }
            }

            Ok(Cow::Owned(res))
        }
    }
}
