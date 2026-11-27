use cosmic_text::{Attrs, AttrsList, AttrsOwned, FontSystem, ShapeBuffer, ShapeGlyph, ShapeWord};
use hashlink::LruCache;
use unicode_bidi::{BidiInfo, Level};

use crate::{GlyphCluster, ShapedRun};

const MIN_REUSABLE_LINE_BYTES: usize = 256;
const MAX_CACHED_WORD_BYTES: usize = 64;
const MAX_CACHED_WORDS: usize = 2048;

pub(crate) fn supports_word_reuse(text: &str) -> bool {
    if text.len() < MIN_REUSABLE_LINE_BYTES || text.contains(['\r', '\n', '\t']) {
        return false;
    }
    // A single level-zero paragraph has exactly one visual-order span.
    let bidi = BidiInfo::new(text, None);
    bidi.paragraphs.len() == 1 && bidi.levels.iter().all(|&level| level == Level::ltr())
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct WordKey {
    text: String,
    attrs: AttrsOwned,
}

pub(crate) struct EditingShapeCache {
    words: LruCache<WordKey, Vec<ShapeGlyph>>,
    scratch: ShapeBuffer,
}

impl Default for EditingShapeCache {
    fn default() -> Self {
        Self { words: LruCache::new(MAX_CACHED_WORDS), scratch: ShapeBuffer::default() }
    }
}

impl EditingShapeCache {
    pub(crate) fn shape_line(
        &mut self,
        text: &str,
        attrs: Attrs<'_>,
        font_size: f32,
        font_system: &mut FontSystem,
    ) -> ShapedRun {
        let attrs_list = AttrsList::new(attrs);
        let mut run = ShapedRun { clusters: Vec::new(), width: 0.0 };
        let mut start = 0;
        // Follow cosmic-text's span partition. Each breakpoint ends one word
        // and its following spaces, which the shaper processes individually.
        for (end, _) in unicode_linebreak::linebreaks(text) {
            let chunk = &text[start..end];
            let word = chunk.trim_end_matches(char::is_whitespace);
            if !word.is_empty() {
                append_word(
                    &mut run,
                    self.shape_word(word, &attrs_list, font_system),
                    start,
                    font_size,
                );
            }
            let whitespace_start = start + word.len();
            for (offset, space) in text[whitespace_start..end].char_indices() {
                let position = whitespace_start + offset;
                let space = &text[position..position + space.len_utf8()];
                append_word(
                    &mut run,
                    self.shape_word(space, &attrs_list, font_system),
                    position,
                    font_size,
                );
            }
            start = end;
        }
        run
    }

    fn shape_word(
        &mut self,
        word: &str,
        attrs: &AttrsList,
        font_system: &mut FontSystem,
    ) -> std::borrow::Cow<'_, [ShapeGlyph]> {
        let mut shape = |scratch: &mut ShapeBuffer| {
            ShapeWord::new_in_buffer(
                scratch,
                font_system,
                word,
                attrs,
                0..word.len(),
                Level::ltr(),
                false,
                cosmic_text::Shaping::Advanced,
            )
            .glyphs
        };
        if word.len() > MAX_CACHED_WORD_BYTES {
            return std::borrow::Cow::Owned(shape(&mut self.scratch));
        }
        let key = WordKey { text: word.to_owned(), attrs: AttrsOwned::new(attrs.defaults()) };
        if self.words.peek(&key).is_none() {
            self.words.insert(key.clone(), shape(&mut self.scratch));
        }
        std::borrow::Cow::Borrowed(
            self.words.get(&key).expect("the word must be cached after lookup or insertion"),
        )
    }
}

fn append_word(
    run: &mut ShapedRun,
    glyphs: std::borrow::Cow<'_, [ShapeGlyph]>,
    byte_start: usize,
    font_size: f32,
) {
    for glyph in glyphs.iter() {
        let advance = glyph.width(font_size);
        run.width += advance;
        run.clusters.push(GlyphCluster {
            byte_range: byte_start + glyph.start..byte_start + glyph.end,
            glyph_id: u32::from(glyph.glyph_id),
            font_id: glyph.font_id,
            advance,
            x_offset: glyph.x_offset * font_size,
            y_offset: glyph.y_offset * font_size,
        });
    }
}
