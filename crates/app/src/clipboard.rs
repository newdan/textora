#[cfg(test)]
use appkit_shell::ClipboardSnapshot;
use appkit_shell::{DocumentClipboard, SystemClipboard};
use ui::core::Clipboard;
use ui::plugin::PastePreference;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PasteRequestKind {
    Smart,
    PlainText,
}

pub fn copy_to_clipboard(text: &str) -> bool {
    SystemClipboard.write_text(text)
}

pub fn paste_from_clipboard() -> Option<String> {
    SystemClipboard.read_text()
}

pub(crate) fn prepare_document_paste(
    clipboard: &mut dyn DocumentClipboard,
    preference: PastePreference,
    request_kind: PasteRequestKind,
) -> Option<String> {
    match (request_kind, preference) {
        (PasteRequestKind::PlainText, _)
        | (PasteRequestKind::Smart, PastePreference::PlainText) => {
            clipboard.read_plain_text().and_then(normalize_document_paste)
        }
        (PasteRequestKind::Smart, PastePreference::SemanticMarkdown) => {
            prepare_semantic_markdown_paste(clipboard)
        }
    }
}

fn normalize_document_paste(text: String) -> Option<String> {
    let normalized = crate::document_view::normalize_paste_text(text.as_bytes());
    if normalized.is_empty() {
        return None;
    }
    Some(
        String::from_utf8(normalized)
            .expect("normalizing valid UTF-8 clipboard text must preserve valid UTF-8"),
    )
}

#[cfg(feature = "markdown")]
fn prepare_semantic_markdown_paste(clipboard: &mut dyn DocumentClipboard) -> Option<String> {
    let snapshot = clipboard.read_snapshot()?;
    let representations = textora_markdown::paste::PasteRepresentations {
        markdown: snapshot.markdown_text.as_deref(),
        html: snapshot.html_text.as_deref(),
        rtf: snapshot.rtf_bytes.as_deref(),
        plain: snapshot.plain_text.as_deref(),
        source_url: snapshot.source_url.as_deref(),
    };
    textora_markdown::paste::prepare_paste(representations)
        .into_text()
        .and_then(normalize_document_paste)
}

#[cfg(not(feature = "markdown"))]
fn prepare_semantic_markdown_paste(clipboard: &mut dyn DocumentClipboard) -> Option<String> {
    clipboard.read_snapshot()?.plain_text.and_then(normalize_document_paste)
}

#[cfg(test)]
pub(crate) struct TestDocumentClipboard {
    snapshot: Option<ClipboardSnapshot>,
    plain_text: Option<String>,
    pub(crate) plain_reads: usize,
    pub(crate) snapshot_reads: usize,
}

#[cfg(test)]
impl TestDocumentClipboard {
    pub(crate) fn empty() -> Self {
        Self { snapshot: None, plain_text: None, plain_reads: 0, snapshot_reads: 0 }
    }

    pub(crate) fn with_plain(plain: &str) -> Self {
        let plain_text = Some(plain.to_owned());
        Self {
            snapshot: Some(ClipboardSnapshot {
                plain_text: plain_text.clone(),
                ..ClipboardSnapshot::default()
            }),
            plain_text,
            plain_reads: 0,
            snapshot_reads: 0,
        }
    }

    pub(crate) fn with_html(html: &str, plain: &str) -> Self {
        let mut clipboard = Self::with_plain(plain);
        clipboard
            .snapshot
            .as_mut()
            .expect("with_plain must create a clipboard snapshot")
            .html_text = Some(html.to_owned());
        clipboard
    }

    pub(crate) fn with_all_formats() -> Self {
        let mut clipboard = Self::with_html("<p><strong>html</strong></p>", "plain\ntext");
        let snapshot =
            clipboard.snapshot.as_mut().expect("with_html must create a clipboard snapshot");
        snapshot.markdown_text = Some("**markdown**".to_owned());
        snapshot.rtf_bytes = Some(br"{\rtf1\b rich}".to_vec());
        clipboard
    }
}

#[cfg(test)]
impl DocumentClipboard for TestDocumentClipboard {
    fn read_plain_text(&mut self) -> Option<String> {
        self.plain_reads += 1;
        self.plain_text.clone()
    }

    fn read_snapshot(&mut self) -> Option<ClipboardSnapshot> {
        self.snapshot_reads += 1;
        self.snapshot.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_view_smart_paste_reads_only_plain_text() {
        let mut clipboard = TestDocumentClipboard::with_all_formats();
        let text = prepare_document_paste(
            &mut clipboard,
            PastePreference::PlainText,
            PasteRequestKind::Smart,
        );

        assert_eq!(text.as_deref(), Some("plain\ntext"));
        assert_eq!(clipboard.plain_reads, 1);
        assert_eq!(clipboard.snapshot_reads, 0);
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn wysiwyg_smart_paste_converts_html() {
        let mut clipboard =
            TestDocumentClipboard::with_html("<p><strong>rich</strong></p>", "rich");
        let text = prepare_document_paste(
            &mut clipboard,
            PastePreference::SemanticMarkdown,
            PasteRequestKind::Smart,
        );

        assert_eq!(text.as_deref(), Some("**rich**"));
        assert_eq!(clipboard.snapshot_reads, 1);
    }

    #[cfg(not(feature = "markdown"))]
    #[test]
    fn semantic_preference_falls_back_to_snapshot_plain_text_without_markdown() {
        let mut clipboard =
            TestDocumentClipboard::with_html("<p><strong>rich</strong></p>", "plain");
        let text = prepare_document_paste(
            &mut clipboard,
            PastePreference::SemanticMarkdown,
            PasteRequestKind::Smart,
        );

        assert_eq!(text.as_deref(), Some("plain"));
        assert_eq!(clipboard.plain_reads, 0);
        assert_eq!(clipboard.snapshot_reads, 1);
    }

    #[test]
    fn forced_plain_paste_ignores_semantic_preference() {
        let mut clipboard = TestDocumentClipboard::with_plain("a\nb\n\nc");
        let text = prepare_document_paste(
            &mut clipboard,
            PastePreference::SemanticMarkdown,
            PasteRequestKind::PlainText,
        );

        assert_eq!(text.as_deref(), Some("a\nb\n\nc"));
        assert_eq!(clipboard.plain_reads, 1);
        assert_eq!(clipboard.snapshot_reads, 0);
    }

    #[test]
    fn platform_clipboard_implementation_is_owned_only_by_the_shared_shell() {
        let app_manifest = include_str!("../Cargo.toml");
        let app_sources = [
            include_str!("commands.rs"),
            include_str!("dispatch/editor.rs"),
            include_str!("document_view/selection.rs"),
            include_str!("workspace_product.rs"),
        ];
        let shell_clipboard_source = include_str!("../../appkit-shell/src/clipboard.rs");

        assert!(!app_manifest.lines().any(|line| line.trim_start().starts_with("arboard")));
        assert!(app_sources.iter().all(|source| !source.contains("arboard::")));
        assert!(shell_clipboard_source.contains("clipboard_rs::ClipboardContext"));
        assert!(!shell_clipboard_source.contains("arboard::"));
    }
}
