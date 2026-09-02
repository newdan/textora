//! Shell-local system clipboard implementation.

use clipboard_rs::{Clipboard, ContentFormat};

const MARKDOWN_FORMAT_ALIASES: &[&str] =
    &["text/markdown", "public.markdown", "net.daringfireball.markdown"];
const HTML_FORMAT_ALIASES: &[&str] = &["HTML Format", "public.html", "text/html"];
const RTF_FORMAT_ALIASES: &[&str] = &["Rich Text Format", "public.rtf", "text/rtf"];
const SOURCE_URL_FORMAT_ALIASES: &[&str] = &["public.url", "text/x-moz-url", "SourceURL"];
const CF_HTML_START_FRAGMENT_MARKER: &str = "<!--StartFragment-->";
const CF_HTML_END_FRAGMENT_MARKER: &str = "<!--EndFragment-->";
const CF_HTML_START_FRAGMENT_HEADER: &str = "StartFragment:";
const CF_HTML_END_FRAGMENT_HEADER: &str = "EndFragment:";
const CF_HTML_SOURCE_URL_HEADER: &str = "SourceURL:";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipboardSnapshot {
    pub markdown_text: Option<String>,
    pub html_text: Option<String>,
    pub rtf_bytes: Option<Vec<u8>>,
    pub plain_text: Option<String>,
    pub source_url: Option<String>,
}

pub trait DocumentClipboard {
    fn read_plain_text(&mut self) -> Option<String>;
    fn read_snapshot(&mut self) -> Option<ClipboardSnapshot>;
}

pub struct SystemClipboard;

impl DocumentClipboard for SystemClipboard {
    fn read_plain_text(&mut self) -> Option<String> {
        try_read_text()
    }

    fn read_snapshot(&mut self) -> Option<ClipboardSnapshot> {
        let context = clipboard_rs::ClipboardContext::new().ok()?;
        let representations = ClipboardContextRepresentations { context: &context };
        snapshot_from(&representations)
    }
}

impl ui::core::Clipboard for SystemClipboard {
    fn read_text(&mut self) -> Option<String> {
        DocumentClipboard::read_plain_text(self)
    }

    fn write_text(&mut self, text: &str) -> bool {
        try_write_text(text)
    }
}

pub(crate) fn try_write_text(text: &str) -> bool {
    clipboard_rs::ClipboardContext::new()
        .and_then(|clipboard| clipboard.set_text(text.to_owned()))
        .is_ok()
}

pub(crate) fn try_read_text() -> Option<String> {
    let context = clipboard_rs::ClipboardContext::new().ok()?;
    let representations = ClipboardContextRepresentations { context: &context };
    non_empty_string(representations.plain_text())
}

trait ClipboardRepresentations {
    fn available_formats(&self) -> Vec<String>;
    fn plain_text(&self) -> Option<String>;
    fn html_text(&self) -> Option<String>;
    fn rtf_bytes(&self) -> Option<Vec<u8>>;
    fn custom_bytes(&self, format: &str) -> Option<Vec<u8>>;
}

struct ClipboardContextRepresentations<'a> {
    context: &'a clipboard_rs::ClipboardContext,
}

impl ClipboardRepresentations for ClipboardContextRepresentations<'_> {
    fn available_formats(&self) -> Vec<String> {
        self.context.available_formats().unwrap_or_default()
    }

    fn plain_text(&self) -> Option<String> {
        self.context.has(ContentFormat::Text).then(|| self.context.get_text().ok()).flatten()
    }

    fn html_text(&self) -> Option<String> {
        self.context.has(ContentFormat::Html).then(|| self.context.get_html().ok()).flatten()
    }

    fn rtf_bytes(&self) -> Option<Vec<u8>> {
        self.context
            .has(ContentFormat::Rtf)
            .then(|| self.context.get_rich_text().ok().map(String::into_bytes))
            .flatten()
    }

    fn custom_bytes(&self, format: &str) -> Option<Vec<u8>> {
        self.context.get_buffer(format).ok()
    }
}

fn snapshot_from(source: &impl ClipboardRepresentations) -> Option<ClipboardSnapshot> {
    let available_formats = source.available_formats();
    let markdown_text = custom_string(source, &available_formats, MARKDOWN_FORMAT_ALIASES);
    let source_url =
        custom_string(source, &available_formats, SOURCE_URL_FORMAT_ALIASES).and_then(first_line);
    let raw_html = custom_string(source, &available_formats, HTML_FORMAT_ALIASES)
        .or_else(|| non_empty_string(source.html_text()));
    let html_source_url = raw_html.as_deref().and_then(extract_cf_html_source_url);
    let html_text = raw_html
        .map(|html| extract_cf_html_fragment(&html).to_owned())
        .and_then(|html| non_empty_string(Some(html)));
    let snapshot = ClipboardSnapshot {
        markdown_text,
        html_text,
        rtf_bytes: custom_bytes(source, &available_formats, RTF_FORMAT_ALIASES)
            .or_else(|| non_empty_bytes(source.rtf_bytes())),
        plain_text: non_empty_string(source.plain_text()),
        source_url: source_url.or(html_source_url),
    };

    snapshot_has_content(&snapshot).then_some(snapshot)
}

fn custom_string(
    source: &impl ClipboardRepresentations,
    available_formats: &[String],
    aliases: &[&str],
) -> Option<String> {
    let bytes = custom_bytes(source, available_formats, aliases)?;
    non_empty_string(String::from_utf8(bytes).ok())
}

fn custom_bytes(
    source: &impl ClipboardRepresentations,
    available_formats: &[String],
    aliases: &[&str],
) -> Option<Vec<u8>> {
    let format = available_formats
        .iter()
        .find(|format| aliases.iter().any(|alias| format.eq_ignore_ascii_case(alias)))?;
    non_empty_bytes(source.custom_bytes(format))
}

fn non_empty_string(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.is_empty())
}

fn non_empty_bytes(value: Option<Vec<u8>>) -> Option<Vec<u8>> {
    value.filter(|bytes| !bytes.is_empty())
}

fn first_line(value: String) -> Option<String> {
    non_empty_string(value.lines().next().map(str::to_owned))
}

fn snapshot_has_content(snapshot: &ClipboardSnapshot) -> bool {
    snapshot.markdown_text.is_some()
        || snapshot.html_text.is_some()
        || snapshot.rtf_bytes.is_some()
        || snapshot.plain_text.is_some()
        || snapshot.source_url.is_some()
}

fn extract_cf_html_fragment(html: &str) -> &str {
    if let Some(fragment) = marker_fragment(html) {
        return fragment;
    }

    numeric_offset_fragment(html).unwrap_or(html)
}

fn marker_fragment(html: &str) -> Option<&str> {
    let fragment_start =
        html.find(CF_HTML_START_FRAGMENT_MARKER)? + CF_HTML_START_FRAGMENT_MARKER.len();
    let fragment_end = html[fragment_start..].find(CF_HTML_END_FRAGMENT_MARKER)? + fragment_start;
    html.get(fragment_start..fragment_end)
}

fn numeric_offset_fragment(html: &str) -> Option<&str> {
    let fragment_start = cf_html_offset(html, CF_HTML_START_FRAGMENT_HEADER)?;
    let fragment_end = cf_html_offset(html, CF_HTML_END_FRAGMENT_HEADER)?;
    (fragment_start < fragment_end).then(|| html.get(fragment_start..fragment_end)).flatten()
}

fn cf_html_offset(html: &str, header: &str) -> Option<usize> {
    html.lines().find_map(|line| line.strip_prefix(header)?.trim().parse::<usize>().ok())
}

fn extract_cf_html_source_url(html: &str) -> Option<String> {
    html.lines()
        .find_map(|line| line.strip_prefix(CF_HTML_SOURCE_URL_HEADER))
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{ClipboardRepresentations, snapshot_from};

    #[derive(Default)]
    struct TestRepresentations {
        custom_formats: BTreeMap<String, Vec<u8>>,
        html_text: Option<String>,
        plain_text: Option<String>,
        rtf_bytes: Option<Vec<u8>>,
    }

    impl TestRepresentations {
        fn new() -> Self {
            Self::default()
        }

        fn with_format(mut self, format: &str, bytes: Vec<u8>) -> Self {
            self.custom_formats.insert(format.to_owned(), bytes);
            self
        }

        fn with_html(mut self, html: &str) -> Self {
            self.html_text = Some(html.to_owned());
            self
        }

        fn with_plain(mut self, plain: &str) -> Self {
            self.plain_text = Some(plain.to_owned());
            self
        }

        fn with_rtf(mut self, rtf: Vec<u8>) -> Self {
            self.rtf_bytes = Some(rtf);
            self
        }
    }

    impl ClipboardRepresentations for TestRepresentations {
        fn available_formats(&self) -> Vec<String> {
            self.custom_formats.keys().cloned().collect()
        }

        fn plain_text(&self) -> Option<String> {
            self.plain_text.clone()
        }

        fn html_text(&self) -> Option<String> {
            self.html_text.clone()
        }

        fn rtf_bytes(&self) -> Option<Vec<u8>> {
            self.rtf_bytes.clone()
        }

        fn custom_bytes(&self, format: &str) -> Option<Vec<u8>> {
            self.custom_formats.get(format).cloned()
        }
    }

    #[test]
    fn snapshot_reads_markdown_html_rtf_plain_and_source_url_from_one_source() {
        let source = TestRepresentations::new()
            .with_format("text/markdown", b"# heading".to_vec())
            .with_format("public.url", b"https://example.com/a/".to_vec())
            .with_html("<p><strong>heading</strong></p>")
            .with_rtf(br"{\rtf1\b heading}".to_vec())
            .with_plain("heading");

        let snapshot = snapshot_from(&source).expect("fixture contains clipboard content");

        assert_eq!(snapshot.markdown_text.as_deref(), Some("# heading"));
        assert_eq!(snapshot.html_text.as_deref(), Some("<p><strong>heading</strong></p>"));
        assert_eq!(snapshot.rtf_bytes.as_deref(), Some(br"{\rtf1\b heading}".as_slice()));
        assert_eq!(snapshot.plain_text.as_deref(), Some("heading"));
        assert_eq!(snapshot.source_url.as_deref(), Some("https://example.com/a/"));
    }

    #[test]
    fn empty_representations_return_none() {
        assert!(snapshot_from(&TestRepresentations::new()).is_none());
    }

    #[test]
    fn cf_html_header_yields_fragment_and_source_url() {
        let payload = "Version:1.0\r\nSourceURL:https://example.com/docs/page\r\n\r\n<!--StartFragment--><p>body</p><!--EndFragment-->";
        let source = TestRepresentations::new().with_html(payload);
        let snapshot = snapshot_from(&source).expect("CF_HTML fixture contains HTML");
        assert_eq!(snapshot.html_text.as_deref(), Some("<p>body</p>"));
        assert_eq!(snapshot.source_url.as_deref(), Some("https://example.com/docs/page"));
    }

    #[test]
    fn snapshot_reads_cf_html_from_each_case_insensitive_native_format() {
        let payload = "Version:1.0\r\nSourceURL:https://example.com/page\r\n\r\n<!--StartFragment--><p>body</p><!--EndFragment-->";
        for format in ["hTmL fOrMaT", "PUBLIC.HTML", "Text/Html"] {
            let source =
                TestRepresentations::new().with_format(format, payload.as_bytes().to_vec());

            let snapshot = snapshot_from(&source).expect("native HTML format contains content");

            assert_eq!(snapshot.html_text.as_deref(), Some("<p>body</p>"));
            assert_eq!(snapshot.source_url.as_deref(), Some("https://example.com/page"));
        }
    }

    #[test]
    fn snapshot_preserves_non_utf8_rtf_from_each_case_insensitive_native_format() {
        let rtf_bytes = vec![b'{', b'\\', b'r', b't', b'f', b'1', 0xff, b'}'];
        for format in ["rIcH tExT fOrMaT", "PUBLIC.RTF", "Text/Rtf"] {
            let source = TestRepresentations::new().with_format(format, rtf_bytes.clone());

            let snapshot = snapshot_from(&source).expect("native RTF format contains content");

            assert_eq!(snapshot.rtf_bytes, Some(rtf_bytes.clone()));
        }
    }

    #[test]
    fn invalid_native_html_utf8_falls_back_to_rich_html() {
        let source = TestRepresentations::new()
            .with_format("HTML Format", vec![0xff])
            .with_html("<p>fallback</p>");

        let snapshot = snapshot_from(&source).expect("rich HTML fallback contains content");

        assert_eq!(snapshot.html_text.as_deref(), Some("<p>fallback</p>"));
    }

    #[test]
    fn mozilla_source_url_uses_only_its_first_line() {
        let source = TestRepresentations::new()
            .with_format("TEXT/X-MOZ-URL", b"https://example.com/page\nPage title".to_vec());

        let snapshot = snapshot_from(&source).expect("Mozilla source URL contains content");

        assert_eq!(snapshot.source_url.as_deref(), Some("https://example.com/page"));
    }

    #[test]
    fn empty_cf_html_fragment_is_not_clipboard_content() {
        let source = TestRepresentations::new().with_html("<!--StartFragment--><!--EndFragment-->");

        assert!(snapshot_from(&source).is_none());
    }

    #[test]
    fn numeric_cf_html_offsets_extract_fragment() {
        let fragment = "<p>numeric</p>";
        let template = "Version:1.0\r\nStartFragment:{start:08}\r\nEndFragment:{end:08}\r\n";
        let header = template.replace("{start:08}", "00000000").replace("{end:08}", "00000000");
        let start = header.len();
        let end = start + fragment.len();
        let payload = format!(
            "Version:1.0\r\nStartFragment:{start:08}\r\nEndFragment:{end:08}\r\n{fragment}"
        );
        let source = TestRepresentations::new().with_html(&payload);

        let snapshot = snapshot_from(&source).expect("numeric CF_HTML offsets contain content");

        assert_eq!(snapshot.html_text.as_deref(), Some(fragment));
    }

    #[test]
    fn marker_fragment_takes_precedence_over_numeric_offsets() {
        let payload = "StartFragment:00000000\r\nEndFragment:00000001\r\n<!--StartFragment--><p>marker</p><!--EndFragment-->";
        let source = TestRepresentations::new().with_html(payload);

        let snapshot = snapshot_from(&source).expect("marker CF_HTML fragment contains content");

        assert_eq!(snapshot.html_text.as_deref(), Some("<p>marker</p>"));
    }

    #[test]
    fn malformed_cf_html_headers_leave_html_untouched() {
        let payload = "StartFragment:not-a-number\r\nEndFragment:00000099\r\n<p>body</p>";
        let source = TestRepresentations::new().with_html(payload);

        let snapshot = snapshot_from(&source).expect("malformed CF_HTML still contains HTML");

        assert_eq!(snapshot.html_text.as_deref(), Some(payload));
    }
}
