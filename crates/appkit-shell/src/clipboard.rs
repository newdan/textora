//! Shell-local system clipboard implementation.

use std::sync::Mutex;

use clipboard_rs::{Clipboard, ClipboardContext, ContentFormat};
use url::Url;

const MARKDOWN_FORMAT_ALIASES: &[&str] =
    &["text/markdown", "public.markdown", "net.daringfireball.markdown"];
const HTML_FORMAT_ALIASES: &[&str] = &["HTML Format", "public.html", "text/html"];
const RTF_FORMAT_ALIASES: &[&str] = &["Rich Text Format", "public.rtf", "text/rtf"];
const PLAIN_TEXT_FORMAT_ALIASES: &[&str] = &["text/plain;charset=utf-8", "text/plain"];
const SOURCE_URL_FORMAT_ALIASES: &[&str] = &["public.url", "text/x-moz-url", "SourceURL"];
const MOZILLA_SOURCE_URL_FORMAT: &str = "text/x-moz-url";
const UTF16_LE_BOM: &[u8] = &[0xff, 0xfe];
const UTF16_ASCII_PREFIX_PAIRS: usize = 3;
const CF_HTML_START_FRAGMENT_MARKER: &str = "<!--StartFragment-->";
const CF_HTML_END_FRAGMENT_MARKER: &str = "<!--EndFragment-->";
const CF_HTML_START_FRAGMENT_HEADER: &str = "StartFragment:";
const CF_HTML_END_FRAGMENT_HEADER: &str = "EndFragment:";
const CF_HTML_SOURCE_URL_HEADER: &str = "SourceURL:";
const STABLE_SNAPSHOT_OBSERVATION_LIMIT: usize = 4;

static SYSTEM_CLIPBOARD_CONTEXT: Mutex<Option<ClipboardContext>> = Mutex::new(None);

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
        with_system_clipboard(|context| {
            let representations = ClipboardContextRepresentations { context };
            snapshot_from(&representations)
        })
        .flatten()
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
    with_system_clipboard(|clipboard| clipboard.set_text(text.to_owned()).is_ok()).unwrap_or(false)
}

pub(crate) fn try_read_text() -> Option<String> {
    with_system_clipboard(|context| {
        let representations = ClipboardContextRepresentations { context };
        best_effort_plain_text_from(&representations)
    })
    .flatten()
}

fn with_system_clipboard<T>(operation: impl FnOnce(&ClipboardContext) -> T) -> Option<T> {
    with_reusable_clipboard_context(&SYSTEM_CLIPBOARD_CONTEXT, ClipboardContext::new, operation)
}

fn with_reusable_clipboard_context<C, T, E>(
    context_slot: &Mutex<Option<C>>,
    initialize: impl FnOnce() -> Result<C, E>,
    operation: impl FnOnce(&C) -> T,
) -> Option<T> {
    let mut context_guard = match context_slot.lock() {
        Ok(context_guard) => context_guard,
        Err(poisoned_context) => {
            let context_guard = poisoned_context.into_inner();
            context_slot.clear_poison();
            context_guard
        }
    };
    if context_guard.is_none() {
        *context_guard = initialize().ok();
    }
    context_guard.as_ref().map(operation)
}

trait ClipboardRepresentations {
    fn available_formats(&self) -> Result<Vec<String>, ClipboardReadError>;
    fn plain_text(&self) -> ClipboardRead<String>;
    fn html_text(&self) -> ClipboardRead<String>;
    fn rtf_bytes(&self) -> ClipboardRead<Vec<u8>>;
    fn custom_bytes(&self, format: &str) -> ClipboardRead<Vec<u8>>;
}

type ClipboardRead<T> = Result<Option<T>, ClipboardReadError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClipboardReadError {
    Backend,
}

struct ClipboardContextRepresentations<'a> {
    context: &'a clipboard_rs::ClipboardContext,
}

#[derive(PartialEq, Eq)]
struct ClipboardObservation {
    format_names: Vec<String>,
    snapshot: Option<ClipboardSnapshot>,
}

impl ClipboardRepresentations for ClipboardContextRepresentations<'_> {
    fn available_formats(&self) -> Result<Vec<String>, ClipboardReadError> {
        self.context.available_formats().map_err(|_| ClipboardReadError::Backend)
    }

    fn plain_text(&self) -> ClipboardRead<String> {
        if !self.context.has(ContentFormat::Text) {
            return Ok(None);
        }
        self.context.get_text().map(Some).map_err(|_| ClipboardReadError::Backend)
    }

    fn html_text(&self) -> ClipboardRead<String> {
        if !self.context.has(ContentFormat::Html) {
            return Ok(None);
        }
        self.context.get_html().map(Some).map_err(|_| ClipboardReadError::Backend)
    }

    fn rtf_bytes(&self) -> ClipboardRead<Vec<u8>> {
        if !self.context.has(ContentFormat::Rtf) {
            return Ok(None);
        }
        self.context
            .get_rich_text()
            .map(|text| Some(text.into_bytes()))
            .map_err(|_| ClipboardReadError::Backend)
    }

    fn custom_bytes(&self, format: &str) -> ClipboardRead<Vec<u8>> {
        self.context.get_buffer(format).map(Some).map_err(|_| ClipboardReadError::Backend)
    }
}

fn snapshot_from(source: &impl ClipboardRepresentations) -> Option<ClipboardSnapshot> {
    // clipboard-rs cannot lock the external owner. Two identical complete
    // observations reject any change that is visible within the fixed budget.
    let mut previous_complete_observation = None;
    for _ in 0..STABLE_SNAPSHOT_OBSERVATION_LIMIT {
        match read_snapshot_observation(source) {
            Ok(current) if previous_complete_observation.as_ref() == Some(&current) => {
                return current.snapshot;
            }
            Ok(current) => previous_complete_observation = Some(current),
            Err(ClipboardReadError::Backend) => previous_complete_observation = None,
        }
    }
    None
}

fn read_snapshot_observation(
    source: &impl ClipboardRepresentations,
) -> Result<ClipboardObservation, ClipboardReadError> {
    let available_formats = source.available_formats()?;
    let snapshot = snapshot_from_available_formats(source, &available_formats)?;
    let mut format_names = available_formats;
    format_names.sort_unstable();
    format_names.dedup();
    Ok(ClipboardObservation { format_names, snapshot })
}

fn snapshot_from_available_formats(
    source: &impl ClipboardRepresentations,
    available_formats: &[String],
) -> ClipboardRead<ClipboardSnapshot> {
    let markdown_text = custom_string(source, available_formats, MARKDOWN_FORMAT_ALIASES)?;
    let source_url = source_url_from(source, available_formats)?;
    let raw_html = match custom_string(source, available_formats, HTML_FORMAT_ALIASES)? {
        Some(html) => Some(html),
        None => non_empty_string(source.html_text()?),
    };
    let html_source_url =
        raw_html.as_deref().and_then(extract_cf_html_source_url).and_then(valid_source_url);
    let html_text = raw_html
        .map(|html| extract_cf_html_fragment(&html).to_owned())
        .and_then(|html| non_empty_string(Some(html)));
    let rtf_bytes = match custom_bytes(source, available_formats, RTF_FORMAT_ALIASES)? {
        Some(bytes) => Some(bytes),
        None => non_empty_bytes(source.rtf_bytes()?),
    };
    let snapshot = ClipboardSnapshot {
        markdown_text,
        html_text,
        rtf_bytes,
        plain_text: plain_text_from(source, available_formats)?,
        source_url: source_url.or(html_source_url),
    };

    Ok(snapshot_has_content(&snapshot).then_some(snapshot))
}

fn custom_string(
    source: &impl ClipboardRepresentations,
    available_formats: &[String],
    aliases: &[&str],
) -> ClipboardRead<String> {
    for format in matching_formats(available_formats, aliases) {
        let Some(bytes) = non_empty_bytes(source.custom_bytes(format)?) else {
            continue;
        };
        let Some(text) =
            String::from_utf8(bytes).ok().and_then(|text| non_empty_string(Some(text)))
        else {
            continue;
        };
        return Ok(Some(text));
    }
    Ok(None)
}

fn custom_bytes(
    source: &impl ClipboardRepresentations,
    available_formats: &[String],
    aliases: &[&str],
) -> ClipboardRead<Vec<u8>> {
    for format in matching_formats(available_formats, aliases) {
        if let Some(bytes) = non_empty_bytes(source.custom_bytes(format)?) {
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

fn matching_formats<'a>(available_formats: &'a [String], aliases: &[&str]) -> Vec<&'a str> {
    aliases
        .iter()
        .filter_map(|alias| {
            available_formats
                .iter()
                .find(|format| format.eq_ignore_ascii_case(alias))
                .map(String::as_str)
        })
        .collect()
}

fn plain_text_from(
    source: &impl ClipboardRepresentations,
    available_formats: &[String],
) -> ClipboardRead<String> {
    if let Some(text) = custom_string(source, available_formats, PLAIN_TEXT_FORMAT_ALIASES)? {
        return Ok(Some(text));
    }
    Ok(non_empty_string(source.plain_text()?))
}

fn best_effort_plain_text_from(source: &impl ClipboardRepresentations) -> Option<String> {
    let available_formats = source.available_formats().unwrap_or_default();
    for format in matching_formats(&available_formats, PLAIN_TEXT_FORMAT_ALIASES) {
        let bytes = source.custom_bytes(format).ok().flatten();
        if let Some(text) =
            bytes.and_then(|bytes| String::from_utf8(bytes).ok()).filter(|s| !s.is_empty())
        {
            return Some(text);
        }
    }
    source.plain_text().ok().flatten().filter(|text| !text.is_empty())
}

fn source_url_from(
    source: &impl ClipboardRepresentations,
    available_formats: &[String],
) -> ClipboardRead<String> {
    for format in matching_formats(available_formats, SOURCE_URL_FORMAT_ALIASES) {
        let Some(bytes) = non_empty_bytes(source.custom_bytes(format)?) else {
            continue;
        };
        let source_url =
            decode_source_url(format, bytes).and_then(first_line).and_then(valid_source_url);
        if source_url.is_some() {
            return Ok(source_url);
        }
    }
    Ok(None)
}

fn decode_source_url(format: &str, bytes: Vec<u8>) -> Option<String> {
    if format.eq_ignore_ascii_case(MOZILLA_SOURCE_URL_FORMAT) && looks_like_utf16_le(&bytes) {
        return decode_utf16_le(&bytes);
    }
    String::from_utf8(bytes).ok()
}

fn looks_like_utf16_le(bytes: &[u8]) -> bool {
    if bytes.starts_with(UTF16_LE_BOM) {
        return true;
    }
    let mut prefix_pairs = bytes.chunks_exact(2).take(UTF16_ASCII_PREFIX_PAIRS);
    (0..UTF16_ASCII_PREFIX_PAIRS)
        .all(|_| prefix_pairs.next().is_some_and(|pair| pair[0].is_ascii_graphic() && pair[1] == 0))
}

fn decode_utf16_le(bytes: &[u8]) -> Option<String> {
    let encoded_text = bytes.strip_prefix(UTF16_LE_BOM).unwrap_or(bytes);
    let mut code_unit_bytes = encoded_text.chunks_exact(2);
    let code_units = code_unit_bytes
        .by_ref()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    code_unit_bytes.remainder().is_empty().then_some(())?;
    String::from_utf16(&code_units).ok()
}

fn non_empty_string(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.is_empty())
}

fn non_empty_bytes(value: Option<Vec<u8>>) -> Option<Vec<u8>> {
    value.filter(|bytes| !bytes.is_empty())
}

fn first_line(value: String) -> Option<String> {
    let text_before_terminator = value.split('\0').next()?;
    non_empty_string(text_before_terminator.lines().next().map(str::to_owned))
}

fn valid_source_url(candidate: String) -> Option<String> {
    let parsed = Url::parse(&candidate).ok()?;
    (matches!(parsed.scheme(), "http" | "https") && parsed.has_host()).then_some(candidate)
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
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use super::{
        ClipboardRead, ClipboardReadError, ClipboardRepresentations, best_effort_plain_text_from,
        snapshot_from, with_reusable_clipboard_context,
    };

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

    fn utf16_le_fixture(value: &str) -> Vec<u8> {
        value.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn utf16_le_fixture_with_bom(value: &str) -> Vec<u8> {
        let mut bytes = super::UTF16_LE_BOM.to_vec();
        bytes.extend(utf16_le_fixture(value));
        bytes
    }

    impl ClipboardRepresentations for TestRepresentations {
        fn available_formats(&self) -> Result<Vec<String>, ClipboardReadError> {
            Ok(self.custom_formats.keys().cloned().collect())
        }

        fn plain_text(&self) -> ClipboardRead<String> {
            Ok(self.plain_text.clone())
        }

        fn html_text(&self) -> ClipboardRead<String> {
            Ok(self.html_text.clone())
        }

        fn rtf_bytes(&self) -> ClipboardRead<Vec<u8>> {
            Ok(self.rtf_bytes.clone())
        }

        fn custom_bytes(&self, format: &str) -> ClipboardRead<Vec<u8>> {
            Ok(self.custom_formats.get(format).cloned())
        }
    }

    struct FailingMarkdownRead {
        failure_count: Cell<usize>,
    }

    impl ClipboardRepresentations for FailingMarkdownRead {
        fn available_formats(&self) -> Result<Vec<String>, ClipboardReadError> {
            Ok(vec!["text/markdown".to_owned()])
        }

        fn plain_text(&self) -> ClipboardRead<String> {
            Ok(Some("plain survives".to_owned()))
        }

        fn html_text(&self) -> ClipboardRead<String> {
            Ok(None)
        }

        fn rtf_bytes(&self) -> ClipboardRead<Vec<u8>> {
            Ok(None)
        }

        fn custom_bytes(&self, _format: &str) -> ClipboardRead<Vec<u8>> {
            self.failure_count.set(self.failure_count.get() + 1);
            Err(ClipboardReadError::Backend)
        }
    }

    #[test]
    fn snapshot_rejects_repeated_markdown_read_errors() {
        let source = FailingMarkdownRead { failure_count: Cell::new(0) };

        assert_eq!(snapshot_from(&source), None);
        assert_eq!(source.failure_count.get(), super::STABLE_SNAPSHOT_OBSERVATION_LIMIT);
    }

    struct FailingPlainRead;

    impl ClipboardRepresentations for FailingPlainRead {
        fn available_formats(&self) -> Result<Vec<String>, ClipboardReadError> {
            Ok(Vec::new())
        }

        fn plain_text(&self) -> ClipboardRead<String> {
            Err(ClipboardReadError::Backend)
        }

        fn html_text(&self) -> ClipboardRead<String> {
            Ok(Some("<p>HTML survives</p>".to_owned()))
        }

        fn rtf_bytes(&self) -> ClipboardRead<Vec<u8>> {
            Ok(None)
        }

        fn custom_bytes(&self, _format: &str) -> ClipboardRead<Vec<u8>> {
            Ok(None)
        }
    }

    #[test]
    fn snapshot_rejects_repeated_plain_text_read_errors_even_with_html() {
        assert_eq!(snapshot_from(&FailingPlainRead), None);
    }

    struct FailingFormatRead;

    impl ClipboardRepresentations for FailingFormatRead {
        fn available_formats(&self) -> Result<Vec<String>, ClipboardReadError> {
            Err(ClipboardReadError::Backend)
        }

        fn plain_text(&self) -> ClipboardRead<String> {
            Ok(Some("plain survives".to_owned()))
        }

        fn html_text(&self) -> ClipboardRead<String> {
            Ok(None)
        }

        fn rtf_bytes(&self) -> ClipboardRead<Vec<u8>> {
            Ok(None)
        }

        fn custom_bytes(&self, _format: &str) -> ClipboardRead<Vec<u8>> {
            Ok(None)
        }
    }

    #[test]
    fn snapshot_rejects_repeated_available_format_errors() {
        assert_eq!(snapshot_from(&FailingFormatRead), None);
    }

    struct RecoveringMarkdownRead {
        observation_count: Cell<usize>,
    }

    impl ClipboardRepresentations for RecoveringMarkdownRead {
        fn available_formats(&self) -> Result<Vec<String>, ClipboardReadError> {
            self.observation_count.set(self.observation_count.get() + 1);
            Ok(vec!["text/markdown".to_owned()])
        }

        fn plain_text(&self) -> ClipboardRead<String> {
            Ok(None)
        }

        fn html_text(&self) -> ClipboardRead<String> {
            Ok(None)
        }

        fn rtf_bytes(&self) -> ClipboardRead<Vec<u8>> {
            Ok(None)
        }

        fn custom_bytes(&self, _format: &str) -> ClipboardRead<Vec<u8>> {
            if self.observation_count.get() == 1 {
                return Err(ClipboardReadError::Backend);
            }
            Ok(Some(b"stable".to_vec()))
        }
    }

    #[test]
    fn snapshot_recovers_after_an_error_and_two_complete_successes() {
        let source = RecoveringMarkdownRead { observation_count: Cell::new(0) };

        let snapshot = snapshot_from(&source).expect("two complete observations become stable");

        assert_eq!(snapshot.markdown_text.as_deref(), Some("stable"));
        assert_eq!(source.observation_count.get(), 3);
    }

    struct InterruptedMarkdownRead {
        observation_count: Cell<usize>,
    }

    impl ClipboardRepresentations for InterruptedMarkdownRead {
        fn available_formats(&self) -> Result<Vec<String>, ClipboardReadError> {
            self.observation_count.set(self.observation_count.get() + 1);
            Ok(vec!["text/markdown".to_owned()])
        }

        fn plain_text(&self) -> ClipboardRead<String> {
            Ok(None)
        }

        fn html_text(&self) -> ClipboardRead<String> {
            Ok(None)
        }

        fn rtf_bytes(&self) -> ClipboardRead<Vec<u8>> {
            Ok(None)
        }

        fn custom_bytes(&self, _format: &str) -> ClipboardRead<Vec<u8>> {
            if self.observation_count.get() == 2 {
                return Err(ClipboardReadError::Backend);
            }
            Ok(Some(b"stable".to_vec()))
        }
    }

    #[test]
    fn read_error_interrupts_consecutive_complete_observations() {
        let source = InterruptedMarkdownRead { observation_count: Cell::new(0) };

        let snapshot = snapshot_from(&source).expect("last two complete observations are stable");

        assert_eq!(snapshot.markdown_text.as_deref(), Some("stable"));
        assert_eq!(source.observation_count.get(), 4);
    }

    struct PlainAliasReadError {
        standard_plain_text: Option<String>,
    }

    impl ClipboardRepresentations for PlainAliasReadError {
        fn available_formats(&self) -> Result<Vec<String>, ClipboardReadError> {
            Ok(vec!["text/plain;charset=utf-8".to_owned(), "text/plain".to_owned()])
        }

        fn plain_text(&self) -> ClipboardRead<String> {
            Ok(self.standard_plain_text.clone())
        }

        fn html_text(&self) -> ClipboardRead<String> {
            Ok(None)
        }

        fn rtf_bytes(&self) -> ClipboardRead<Vec<u8>> {
            Ok(None)
        }

        fn custom_bytes(&self, format: &str) -> ClipboardRead<Vec<u8>> {
            if format.eq_ignore_ascii_case("text/plain;charset=utf-8") {
                return Err(ClipboardReadError::Backend);
            }
            Ok(format.eq_ignore_ascii_case("text/plain").then(|| b"generic".to_vec()))
        }
    }

    #[test]
    fn snapshot_does_not_hide_a_plain_alias_backend_error() {
        let source = PlainAliasReadError { standard_plain_text: None };

        assert_eq!(snapshot_from(&source), None);
    }

    #[test]
    fn plain_only_read_skips_a_failed_alias_and_uses_the_next_alias() {
        let source = PlainAliasReadError { standard_plain_text: None };

        assert_eq!(best_effort_plain_text_from(&source).as_deref(), Some("generic"));
    }

    struct FailedPlainAliasesWithStandardText;

    impl ClipboardRepresentations for FailedPlainAliasesWithStandardText {
        fn available_formats(&self) -> Result<Vec<String>, ClipboardReadError> {
            Ok(vec!["text/plain;charset=utf-8".to_owned(), "text/plain".to_owned()])
        }

        fn plain_text(&self) -> ClipboardRead<String> {
            Ok(Some("standard".to_owned()))
        }

        fn html_text(&self) -> ClipboardRead<String> {
            Ok(None)
        }

        fn rtf_bytes(&self) -> ClipboardRead<Vec<u8>> {
            Ok(None)
        }

        fn custom_bytes(&self, _format: &str) -> ClipboardRead<Vec<u8>> {
            Err(ClipboardReadError::Backend)
        }
    }

    #[test]
    fn plain_only_read_falls_back_to_standard_text_after_alias_errors() {
        assert_eq!(
            best_effort_plain_text_from(&FailedPlainAliasesWithStandardText).as_deref(),
            Some("standard")
        );
    }

    struct SwitchAfterMarkdownRead {
        switched_to_second_copy: Cell<bool>,
    }

    impl ClipboardRepresentations for SwitchAfterMarkdownRead {
        fn available_formats(&self) -> Result<Vec<String>, ClipboardReadError> {
            Ok(vec!["text/markdown".to_owned(), "text/html".to_owned()])
        }

        fn plain_text(&self) -> ClipboardRead<String> {
            Ok(Some("second".to_owned()))
        }

        fn html_text(&self) -> ClipboardRead<String> {
            Ok(None)
        }

        fn rtf_bytes(&self) -> ClipboardRead<Vec<u8>> {
            Ok(None)
        }

        fn custom_bytes(&self, format: &str) -> ClipboardRead<Vec<u8>> {
            if format.eq_ignore_ascii_case("text/markdown")
                && !self.switched_to_second_copy.replace(true)
            {
                return Ok(Some(b"first".to_vec()));
            }

            Ok(match format.to_ascii_lowercase().as_str() {
                "text/markdown" => Some(b"second".to_vec()),
                "text/html" => Some(b"<p>second</p>".to_vec()),
                _ => None,
            })
        }
    }

    struct AlternatingRepresentations {
        observation_count: Cell<usize>,
    }

    impl ClipboardRepresentations for AlternatingRepresentations {
        fn available_formats(&self) -> Result<Vec<String>, ClipboardReadError> {
            self.observation_count.set(self.observation_count.get() + 1);
            Ok(vec!["text/markdown".to_owned()])
        }

        fn plain_text(&self) -> ClipboardRead<String> {
            Ok(None)
        }

        fn html_text(&self) -> ClipboardRead<String> {
            Ok(None)
        }

        fn rtf_bytes(&self) -> ClipboardRead<Vec<u8>> {
            Ok(None)
        }

        fn custom_bytes(&self, format: &str) -> ClipboardRead<Vec<u8>> {
            Ok(format.eq_ignore_ascii_case("text/markdown").then(|| {
                if self.observation_count.get().is_multiple_of(2) {
                    b"second".to_vec()
                } else {
                    b"first".to_vec()
                }
            }))
        }
    }

    struct AlternatingFormatRepresentations {
        observation_count: Cell<usize>,
    }

    impl ClipboardRepresentations for AlternatingFormatRepresentations {
        fn available_formats(&self) -> Result<Vec<String>, ClipboardReadError> {
            self.observation_count.set(self.observation_count.get() + 1);
            let changing_format = if self.observation_count.get().is_multiple_of(2) {
                "application/x-second-copy"
            } else {
                "application/x-first-copy"
            };
            Ok(vec!["text/markdown".to_owned(), changing_format.to_owned()])
        }

        fn plain_text(&self) -> ClipboardRead<String> {
            Ok(None)
        }

        fn html_text(&self) -> ClipboardRead<String> {
            Ok(None)
        }

        fn rtf_bytes(&self) -> ClipboardRead<Vec<u8>> {
            Ok(None)
        }

        fn custom_bytes(&self, format: &str) -> ClipboardRead<Vec<u8>> {
            Ok(format.eq_ignore_ascii_case("text/markdown").then(|| b"same".to_vec()))
        }
    }

    #[test]
    fn reusable_context_initializes_once_across_operations() {
        let context = Mutex::new(None);
        let mut initialization_count = 0;

        for expected_value in [1, 2, 3] {
            let actual_value = with_reusable_clipboard_context(
                &context,
                || {
                    initialization_count += 1;
                    Ok::<_, ()>(41)
                },
                |value| value + expected_value,
            );

            assert_eq!(actual_value, Some(41 + expected_value));
        }

        assert_eq!(initialization_count, 1);
    }

    #[test]
    fn reusable_context_retries_after_initialization_failure() {
        let context = Mutex::new(None);

        assert_eq!(
            with_reusable_clipboard_context(&context, || Err::<usize, _>(()), |value| *value),
            None
        );
        assert_eq!(
            with_reusable_clipboard_context(&context, || Ok::<_, ()>(7), |value| *value),
            Some(7)
        );
    }

    #[test]
    fn reusable_context_recovers_a_poisoned_mutex() {
        let context = Arc::new(Mutex::new(Some(7)));
        let panic_context = Arc::clone(&context);
        let panic_result = std::thread::spawn(move || {
            let _guard = panic_context.lock().expect("test mutex starts healthy");
            panic!("poison the test mutex");
        })
        .join();
        assert!(panic_result.is_err());

        let value =
            with_reusable_clipboard_context(context.as_ref(), || Ok::<_, ()>(9), |value| *value);

        assert_eq!(value, Some(7));
        assert!(!context.is_poisoned());
    }

    #[test]
    fn stable_snapshot_discards_a_mixed_read_then_accepts_the_stable_copy() {
        let source = SwitchAfterMarkdownRead { switched_to_second_copy: Cell::new(false) };

        let snapshot = snapshot_from(&source).expect("the second copy becomes stable");

        assert_eq!(snapshot.markdown_text.as_deref(), Some("second"));
        assert_eq!(snapshot.html_text.as_deref(), Some("<p>second</p>"));
        assert_eq!(snapshot.plain_text.as_deref(), Some("second"));
    }

    #[test]
    fn stable_snapshot_rejects_a_clipboard_that_keeps_changing() {
        let source = AlternatingRepresentations { observation_count: Cell::new(0) };

        assert_eq!(snapshot_from(&source), None);
        assert_eq!(source.observation_count.get(), super::STABLE_SNAPSHOT_OBSERVATION_LIMIT);
    }

    #[test]
    fn stable_snapshot_rejects_changing_formats_even_when_fields_match() {
        let source = AlternatingFormatRepresentations { observation_count: Cell::new(0) };

        assert_eq!(snapshot_from(&source), None);
    }

    #[test]
    fn snapshot_reads_plain_text_from_the_linux_utf8_mime_target() {
        let source = TestRepresentations::new()
            .with_format("text/plain;charset=UTF-8", b"mime plain".to_vec());

        let snapshot = snapshot_from(&source).expect("plain MIME target contains content");

        assert_eq!(snapshot.plain_text.as_deref(), Some("mime plain"));
    }

    #[test]
    fn plain_only_reads_the_linux_mime_target_without_content_format_text() {
        let source = TestRepresentations::new()
            .with_format("text/plain;charset=UTF-8", b"mime plain".to_vec());

        let plain_text = best_effort_plain_text_from(&source);

        assert_eq!(plain_text.as_deref(), Some("mime plain"));
    }

    #[test]
    fn plain_only_prefers_the_utf8_mime_alias_over_generic_text_plain() {
        let source = TestRepresentations::new()
            .with_format("text/plain", b"generic".to_vec())
            .with_format("text/plain;charset=utf-8", b"utf8-specific".to_vec());

        let plain_text = best_effort_plain_text_from(&source);

        assert_eq!(plain_text.as_deref(), Some("utf8-specific"));
    }

    #[test]
    fn invalid_plain_mime_utf8_falls_back_to_content_format_text() {
        let source = TestRepresentations::new()
            .with_format("text/plain;charset=utf-8", vec![0xff])
            .with_plain("fallback");

        let plain_text = best_effort_plain_text_from(&source);

        assert_eq!(plain_text.as_deref(), Some("fallback"));
    }

    #[test]
    fn empty_specific_plain_alias_falls_back_to_generic_plain_alias() {
        let source = TestRepresentations::new()
            .with_format("text/plain;charset=utf-8", Vec::new())
            .with_format("text/plain", b"generic".to_vec());

        let plain_text = best_effort_plain_text_from(&source);

        assert_eq!(plain_text.as_deref(), Some("generic"));
    }

    #[test]
    fn invalid_specific_plain_alias_falls_back_to_generic_plain_alias() {
        let source = TestRepresentations::new()
            .with_format("text/plain;charset=utf-8", vec![0xff])
            .with_format("text/plain", b"generic".to_vec());

        let plain_text = best_effort_plain_text_from(&source);

        assert_eq!(plain_text.as_deref(), Some("generic"));
    }

    #[test]
    fn mozilla_source_url_decodes_utf16_le_with_bom_title_and_terminator() {
        let source = TestRepresentations::new().with_format(
            "text/x-moz-url",
            utf16_le_fixture_with_bom("https://example.com/bom\r\nPage title\0"),
        );

        let snapshot = snapshot_from(&source).expect("UTF-16 Mozilla URL contains content");

        assert_eq!(snapshot.source_url.as_deref(), Some("https://example.com/bom"));
    }

    #[test]
    fn mozilla_source_url_decodes_utf16_le_without_bom() {
        let source = TestRepresentations::new().with_format(
            "TEXT/X-MOZ-URL",
            utf16_le_fixture("https://example.com/no-bom\nPage title\0"),
        );

        let snapshot = snapshot_from(&source).expect("UTF-16 Mozilla URL contains content");

        assert_eq!(snapshot.source_url.as_deref(), Some("https://example.com/no-bom"));
    }

    #[test]
    fn utf8_mozilla_and_public_url_formats_remain_supported() {
        for (format, bytes) in [
            ("text/x-moz-url", b"https://example.com/moz\nTitle".as_slice()),
            ("public.url", b"https://example.com/public".as_slice()),
        ] {
            let source = TestRepresentations::new().with_format(format, bytes.to_vec());

            let snapshot = snapshot_from(&source).expect("UTF-8 source URL contains content");

            let expected = if format == "public.url" {
                "https://example.com/public"
            } else {
                "https://example.com/moz"
            };
            assert_eq!(snapshot.source_url.as_deref(), Some(expected));
        }
    }

    #[test]
    fn malformed_mozilla_utf16_does_not_override_cf_html_source_url() {
        let cf_html = "SourceURL:https://example.com/from-html\r\n<!--StartFragment--><p>body</p><!--EndFragment-->";
        let malformed_utf16 = vec![b'h', 0, b't', 0, b't', 0, b'p'];
        let source = TestRepresentations::new()
            .with_format("text/x-moz-url", malformed_utf16)
            .with_format("HTML Format", cf_html.as_bytes().to_vec());

        let snapshot = snapshot_from(&source).expect("CF_HTML contains valid content");

        assert_eq!(snapshot.source_url.as_deref(), Some("https://example.com/from-html"));
    }

    #[test]
    fn semantically_invalid_mozilla_url_does_not_override_cf_html_source_url() {
        let cf_html = "SourceURL:https://example.com/from-html\r\n<!--StartFragment--><p>body</p><!--EndFragment-->";
        let source = TestRepresentations::new()
            .with_format("text/x-moz-url", b"javascript:alert(1)\nTitle".to_vec())
            .with_format("HTML Format", cf_html.as_bytes().to_vec());

        let snapshot = snapshot_from(&source).expect("CF_HTML contains valid content");

        assert_eq!(snapshot.source_url.as_deref(), Some("https://example.com/from-html"));
    }

    #[test]
    fn source_url_accepts_only_absolute_http_or_https_urls() {
        for invalid_url in ["relative/path", "ftp://example.com/file", "https://"] {
            let payload = format!(
                "SourceURL:{invalid_url}\r\n<!--StartFragment--><p>body</p><!--EndFragment-->"
            );
            let source =
                TestRepresentations::new().with_format("HTML Format", payload.as_bytes().to_vec());

            let snapshot = snapshot_from(&source).expect("HTML remains valid clipboard content");

            assert_eq!(snapshot.source_url, None, "accepted invalid URL: {invalid_url}");
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
    fn snapshot_tries_later_aliases_after_empty_or_invalid_values() {
        let source = TestRepresentations::new()
            .with_format("text/markdown", Vec::new())
            .with_format("public.markdown", b"# later".to_vec())
            .with_format("HTML Format", vec![0xff])
            .with_format("public.html", b"<p>later</p>".to_vec())
            .with_format("Rich Text Format", Vec::new())
            .with_format("public.rtf", br"{\rtf1 later}".to_vec())
            .with_format("text/plain;charset=utf-8", vec![0xff])
            .with_format("text/plain", b"later".to_vec())
            .with_format("public.url", b"relative/path".to_vec())
            .with_format("text/x-moz-url", b"https://example.com/later\nTitle".to_vec());

        let snapshot = snapshot_from(&source).expect("later aliases contain complete content");

        assert_eq!(snapshot.markdown_text.as_deref(), Some("# later"));
        assert_eq!(snapshot.html_text.as_deref(), Some("<p>later</p>"));
        assert_eq!(snapshot.rtf_bytes.as_deref(), Some(br"{\rtf1 later}".as_slice()));
        assert_eq!(snapshot.plain_text.as_deref(), Some("later"));
        assert_eq!(snapshot.source_url.as_deref(), Some("https://example.com/later"));
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
