use std::fs::File;
use std::io::{self, Read as _, Write as _};
use std::mem::MaybeUninit;

use stdext::arena::scratch_arena;

use crate::buffer::text_buffer::TextBuffer;
use crate::document::WriteableDocument;
use crate::helpers::CoordType;
use crate::helpers::file_read_uninit;
use crate::simd;
use stdext::slice_as_uninit_mut;

use crate::buffer::text_buffer::IoResult;
use crate::icu;

use crate::helpers::KIBI;

const BOM_MAX_LEN: usize = 4;

fn detect_bom(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 4 {
        if bytes.starts_with(b"\xFF\xFE\x00\x00") {
            return Some("UTF-32LE");
        }
        if bytes.starts_with(b"\x00\x00\xFE\xFF") {
            return Some("UTF-32BE");
        }
        if bytes.starts_with(b"\x84\x31\x95\x33") {
            return Some("GB18030");
        }
    }
    if bytes.len() >= 3 && bytes.starts_with(b"\xEF\xBB\xBF") {
        return Some("UTF-8 BOM");
    }
    if bytes.len() >= 2 {
        if bytes.starts_with(b"\xFF\xFE") {
            return Some("UTF-16LE");
        }
        if bytes.starts_with(b"\xFE\xFF") {
            return Some("UTF-16BE");
        }
    }
    None
}

impl TextBuffer {
    /// Copies the contents of the buffer into a string.
    pub fn save_as_string(&mut self, dst: &mut dyn WriteableDocument) {
        self.buffer.copy_into(dst);
        self.mark_as_clean();
    }

    /// Reads a file from disk into the text buffer, detecting encoding and BOM.
    pub fn read_file(&mut self, file: &mut File, encoding: Option<&'static str>) -> IoResult<()> {
        let scratch = scratch_arena(None);
        let buf = scratch.alloc_uninit_array();
        let mut first_chunk_len = 0;
        let mut read = 0;

        // Read enough bytes to detect the BOM.
        while first_chunk_len < BOM_MAX_LEN {
            read = file_read_uninit(file, &mut buf[first_chunk_len..])?;
            if read == 0 {
                break;
            }
            first_chunk_len += read;
        }

        if let Some(encoding) = encoding {
            self.encoding = encoding;
        } else {
            let bom = detect_bom(unsafe { buf[..first_chunk_len].assume_init_ref() });
            self.encoding = bom.unwrap_or("UTF-8");
        }

        self.buffer.clear();

        let done = read == 0;
        if self.encoding == "UTF-8" {
            self.read_file_as_utf8(file, buf, first_chunk_len, done)?;
        } else {
            self.read_file_with_icu(file, buf, first_chunk_len, done)?;
        }

        {
            let chunk = self.read_forward(0);
            let mut offset = 0;
            let mut lines = 0;
            let mut crlf_count = 0;
            let mut tab_indentations = 0;
            let mut space_indentations = 0;
            let mut space_indentation_sizes = [0; 7];

            loop {
                if offset < chunk.len() && chunk[offset] == b'\t' {
                    tab_indentations += 1;
                } else {
                    let space_indentation =
                        chunk[offset..].iter().take(9).take_while(|&&c| c == b' ').count();

                    if (2..=8).contains(&space_indentation) {
                        space_indentations += 1;
                        space_indentation_sizes[space_indentation - 2] += 1;
                        if space_indentation & 4 != 0 {
                            space_indentation_sizes[0] += 1;
                        }
                        if space_indentation == 6 || space_indentation == 8 {
                            space_indentation_sizes[space_indentation / 2 - 2] += 1;
                        }
                    }
                }

                (offset, lines) = simd::lines_fwd(chunk, offset, lines, lines + 1);

                if offset >= 2 && &chunk[offset - 2..offset] == b"\r\n" {
                    crlf_count += 1;
                }

                if offset >= chunk.len() || lines >= 1000 {
                    break;
                }
            }

            let newlines_are_crlf = if lines == 0 { cfg!(windows) } else { crlf_count > lines / 2 };

            let indent_with_tabs = tab_indentations > space_indentations;
            let tab_size = if indent_with_tabs {
                4
            } else {
                let mut max = 1;
                let mut tab_size = 4;
                for (i, &count) in space_indentation_sizes.iter().enumerate() {
                    if count >= max {
                        max = count;
                        tab_size = i as CoordType + 2;
                    }
                }
                tab_size
            };

            if offset < chunk.len() {
                (_, lines) = simd::lines_fwd(chunk, offset, lines, CoordType::MAX);
            }

            let final_newline = chunk.ends_with(b"\n");

            self.stats.logical_lines = lines + 1;
            self.stats.visual_lines = self.stats.logical_lines;
            self.newlines_are_crlf = newlines_are_crlf;
            self.insert_final_newline = final_newline;
            self.indent_with_tabs = indent_with_tabs;
            self.tab_size = tab_size;
        }

        self.recalc_after_content_swap();
        Ok(())
    }

    fn read_file_as_utf8(
        &mut self,
        file: &mut File,
        buf: &mut [MaybeUninit<u8>; 4 * KIBI],
        first_chunk_len: usize,
        done: bool,
    ) -> io::Result<()> {
        {
            let mut first_chunk = unsafe { buf[..first_chunk_len].assume_init_ref() };
            if first_chunk.starts_with(b"\xEF\xBB\xBF") {
                first_chunk = &first_chunk[3..];
                self.encoding = "UTF-8 BOM";
            }

            self.buffer.replace(0..0, first_chunk);
        }

        if done {
            return Ok(());
        }

        let mut chunk_size = 128 * KIBI;
        let mut extra_chunk_size = 128 * KIBI;

        if let Ok(m) = file.metadata() {
            let len = m.len() as usize;
            chunk_size = len.saturating_sub(first_chunk_len);
            extra_chunk_size = 4 * KIBI;
        }

        loop {
            let gap = self.buffer.allocate_gap(self.text_length(), chunk_size, 0);
            if gap.is_empty() {
                break;
            }

            let read = file.read(gap)?;
            if read == 0 {
                break;
            }

            self.buffer.commit_gap(read);
            chunk_size = extra_chunk_size;
        }

        Ok(())
    }

    fn read_file_with_icu(
        &mut self,
        file: &mut File,
        buf: &mut [MaybeUninit<u8>; 4 * KIBI],
        first_chunk_len: usize,
        mut done: bool,
    ) -> IoResult<()> {
        let scratch = scratch_arena(None);
        let pivot_buffer = scratch.alloc_uninit_slice(4 * KIBI);
        let mut c = icu::Converter::new(pivot_buffer, self.encoding, "UTF-8")?;
        let mut first_chunk = unsafe { buf[..first_chunk_len].assume_init_ref() };

        while !first_chunk.is_empty() {
            let off = self.text_length();
            let gap = self.buffer.allocate_gap(off, 8 * KIBI, 0);
            let (input_advance, mut output_advance) =
                c.convert(first_chunk, slice_as_uninit_mut(gap))?;

            if off == 0 {
                let written = &mut gap[..output_advance];
                if written.starts_with(b"\xEF\xBB\xBF") {
                    written.copy_within(3.., 0);
                    output_advance -= 3;
                }
            }

            self.buffer.commit_gap(output_advance);
            first_chunk = &first_chunk[input_advance..];
        }

        let mut buf_len = 0;

        loop {
            if !done {
                let read = file_read_uninit(file, &mut buf[buf_len..])?;
                buf_len += read;
                done = read == 0;
            }

            let gap = self.buffer.allocate_gap(self.text_length(), 8 * KIBI, 0);
            if gap.is_empty() {
                break;
            }

            let read = unsafe { buf[..buf_len].assume_init_ref() };
            let (input_advance, output_advance) = c.convert(read, slice_as_uninit_mut(gap))?;

            self.buffer.commit_gap(output_advance);

            let flush = done && buf_len == 0;
            buf_len -= input_advance;
            buf.copy_within(input_advance.., 0);

            if flush {
                break;
            }
        }

        Ok(())
    }

    /// Writes the text buffer contents to a file, handling BOM and encoding.
    pub fn write_file(&mut self, file: &mut File) -> IoResult<()> {
        let mut offset = 0;

        if self.encoding.starts_with("UTF-8") {
            if self.encoding == "UTF-8 BOM" {
                file.write_all(b"\xEF\xBB\xBF")?;
            }
            loop {
                let chunk = self.read_forward(offset);
                if chunk.is_empty() {
                    break;
                }
                file.write_all(chunk)?;
                offset += chunk.len();
            }
        } else {
            self.write_file_with_icu(file)?;
        }

        self.mark_as_clean();
        Ok(())
    }

    fn write_file_with_icu(&mut self, file: &mut File) -> IoResult<()> {
        let scratch = scratch_arena(None);
        let pivot_buffer = scratch.alloc_uninit_slice(4 * KIBI);
        let buf = scratch.alloc_uninit_slice(4 * KIBI);
        let mut c = icu::Converter::new(pivot_buffer, "UTF-8", self.encoding)?;
        let mut offset = 0;

        if self.encoding.starts_with("UTF-16")
            || self.encoding.starts_with("UTF-32")
            || self.encoding == "GB18030"
        {
            let (_, output_advance) = c.convert(b"\xEF\xBB\xBF", buf)?;
            let chunk = unsafe { buf[..output_advance].assume_init_ref() };
            file.write_all(chunk)?;
        }

        loop {
            let chunk = self.read_forward(offset);
            let (input_advance, output_advance) = c.convert(chunk, buf)?;
            let chunk = unsafe { buf[..output_advance].assume_init_ref() };

            file.write_all(chunk)?;
            offset += input_advance;

            if chunk.is_empty() {
                break;
            }
        }

        Ok(())
    }
}
