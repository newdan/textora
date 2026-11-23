use std::ops::{Deref, DerefMut};
use std::path::Path;

use appkit_core::document::DocumentModel;
use appkit_shell::document_presentation::DocumentPresentation;

pub(crate) struct DocumentFixture {
    pub(crate) model: DocumentModel,
    pub(crate) presentation: DocumentPresentation,
}

impl DocumentFixture {
    pub(crate) fn new(lines: Vec<String>, visible_rows: usize, viewport_height: f64) -> Self {
        Self::from_model(DocumentModel::from_lines(&lines), visible_rows, viewport_height)
    }

    pub(crate) fn from_file(
        path: &Path,
        visible_rows: usize,
        viewport_height: f64,
    ) -> Result<Self, String> {
        DocumentModel::from_file(path)
            .map(|model| Self::from_model(model, visible_rows, viewport_height))
    }

    pub(crate) fn from_external_content(
        path: &Path,
        content: &str,
        visible_rows: usize,
        viewport_height: f64,
    ) -> Self {
        Self::from_model(
            DocumentModel::from_external_content(path, content),
            visible_rows,
            viewport_height,
        )
    }

    fn from_model(model: DocumentModel, visible_rows: usize, viewport_height: f64) -> Self {
        Self { model, presentation: DocumentPresentation::new(visible_rows, viewport_height) }
    }

    pub(crate) fn into_parts(self) -> (DocumentModel, DocumentPresentation) {
        (self.model, self.presentation)
    }

    pub(crate) fn document_lines(&self) -> Vec<String> {
        (0..self.line_count())
            .map(|line| {
                let bytes = self
                    .document_line_bytes(line)
                    .expect("an indexed fixture line must have a byte range");
                String::from_utf8(bytes.into_owned()).expect("fixture lines must be valid UTF-8")
            })
            .collect()
    }
}

impl Deref for DocumentFixture {
    type Target = DocumentModel;

    fn deref(&self) -> &Self::Target {
        &self.model
    }
}

impl DerefMut for DocumentFixture {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.model
    }
}
