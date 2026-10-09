use std::sync::Arc;

use ui::core::paint::RasterImage;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EmbeddedKind {
    InlineMath,
    DisplayMath,
    Mermaid,
}

#[derive(Clone, Copy, Debug)]
pub struct EmbeddedRequest<'a> {
    pub kind: EmbeddedKind,
    pub source: &'a str,
    pub font_size: f32,
    pub foreground: [u8; 4],
    pub background: [u8; 4],
}

#[derive(Clone, Debug)]
pub struct EmbeddedImage {
    pub image: Arc<RasterImage>,
    pub baseline: f32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EmbeddedError {
    Unsupported,
}

impl std::fmt::Display for EmbeddedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "embedded Markdown rendering is disabled in this build")
    }
}

impl std::error::Error for EmbeddedError {}

pub fn render_embedded(_request: EmbeddedRequest<'_>) -> Result<EmbeddedImage, EmbeddedError> {
    Err(EmbeddedError::Unsupported)
}
