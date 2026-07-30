/// Character class for word-boundary detection.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CharClass {
    Word,
    Whitespace,
    Punctuation,
}

pub fn classify(ch: char) -> CharClass {
    if ch.is_alphanumeric() || ch == '_' {
        CharClass::Word
    } else if ch.is_whitespace() {
        CharClass::Whitespace
    } else {
        CharClass::Punctuation
    }
}
