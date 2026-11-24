//! File loading and saving utilities for the editor.
//!
//! Handles opening files, detecting line endings, rejecting binary files,
//! and handling BOM markers.
//!
//! ## Loading
//! `load_file` reads directly into a GapBuffer using `allocate_gap`/`commit_gap`
//! for zero-copy efficiency.
//!
//! ## Saving
//! `save_file` writes GapBuffer content atomically (temp file + rename), preserving
//! the original file's line endings (LF/CRLF/CR) and UTF-8 BOM. File permissions are
//! carried over from the original file.
use crate::buffer::GapBuffer;
use crate::disk_revision::{DiskRevision, read_disk_revision, revision_from_hash};
use crate::document::ReadableDocument;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Line ending style detected in a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    /// Unix-style (\n)
    Lf,
    /// Windows-style (\r\n)
    Crlf,
    /// Old Mac-style (\r only)
    Cr,
    /// Mixed line endings
    Mixed,
    /// No line endings found (single line or empty)
    None,
}

/// Errors that can occur when opening a file.
#[derive(Debug)]
pub enum FileError {
    Io(io::Error),
    Binary,
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileError::Io(e) => write!(f, "IO error: {e}"),
            FileError::Binary => write!(f, "binary file detected"),
        }
    }
}

impl std::error::Error for FileError {}

impl From<io::Error> for FileError {
    fn from(e: io::Error) -> Self {
        FileError::Io(e)
    }
}

/// Strip UTF-8 BOM from the beginning of bytes if present.
pub fn strip_bom(bytes: &[u8]) -> (bool, &[u8]) {
    if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        (true, &bytes[3..])
    } else {
        (false, bytes)
    }
}

/// Metadata about a loaded file.
pub struct FileMetadata {
    /// Detected line ending style.
    pub line_ending: LineEnding,
    /// Whether a UTF-8 BOM was present.
    pub had_bom: bool,
    /// 若加载时发生了编码转码，记录原始编码名称（如 "GBK", "Shift_JIS"）。
    /// UTF-8 / ASCII 文件此字段为 None。
    pub original_encoding: Option<&'static str>,
}

/// 检测原始编码并转码为 UTF-8。
///
/// 检测优先级：
/// 1. UTF-16 BOM（LE / BE）
/// 2. 无 BOM 时优先尝试 GBK/GB18030，再尝试 Big5、Shift_JIS、EUC-JP 和 EUC-KR
/// 3. 兜底：ISO-8859-15（永不报错）
///
/// 返回 (转码后 UTF-8 字节, 原始编码名称)。
fn transcode_to_utf8(raw: &[u8]) -> (Vec<u8>, &'static str) {
    // BOM 检测：UTF-16 LE / BE
    if raw.len() >= 2 {
        if raw[0] == 0xFF && raw[1] == 0xFE {
            let (decoded, _, had_errors) = encoding_rs::UTF_16LE.decode(raw);
            if !had_errors {
                return (decoded.into_owned().into_bytes(), "UTF-16LE");
            }
        }
        if raw[0] == 0xFE && raw[1] == 0xFF {
            let (decoded, _, had_errors) = encoding_rs::UTF_16BE.decode(raw);
            if !had_errors {
                return (decoded.into_owned().into_bytes(), "UTF-16BE");
            }
        }
    }

    if let (decoded, _, false) = encoding_rs::GB18030.decode(raw) {
        // The GBK and GB18030 decoders share the same implementation. Only a
        // four-byte sequence distinguishes data that needs the extended encoding.
        let name = if has_gb18030_four_byte_sequence(raw) { "GB18030" } else { "GBK" };
        return (decoded.into_owned().into_bytes(), name);
    }

    let candidates: &[(&str, &encoding_rs::Encoding)] = &[
        ("Big5", encoding_rs::BIG5),
        ("Shift_JIS", encoding_rs::SHIFT_JIS),
        ("EUC-JP", encoding_rs::EUC_JP),
        ("EUC-KR", encoding_rs::EUC_KR),
    ];

    for &(name, encoding) in candidates {
        let (decoded, _, had_errors) = encoding.decode(raw);
        if !had_errors {
            return (decoded.into_owned().into_bytes(), name);
        }
    }

    // 兜底：ISO-8859-15（encoding_rs 不提供 ISO-8859-1，二者仅 8 个码点差异）
    // ISO-8859-15 永不报错，确保始终有输出
    let (decoded, _, _) = encoding_rs::ISO_8859_15.decode(raw);
    (decoded.into_owned().into_bytes(), "ISO-8859-15")
}

fn has_gb18030_four_byte_sequence(bytes: &[u8]) -> bool {
    let mut bytes = bytes.iter().copied();
    while let Some(lead) = bytes.next() {
        // ASCII and the single-byte Windows euro sign have no trail byte.
        if lead > 0x80 && bytes.next().is_some_and(|trail| trail.is_ascii_digit()) {
            return true;
        }
    }
    false
}

/// Size of the first chunk read for binary detection, BOM stripping, and UTF-8 check.
const FIRST_CHUNK_SIZE: usize = 8192;
/// Size of subsequent read chunks during streaming load.
const READ_CHUNK_SIZE: usize = 65536;

// ── Streaming UTF-8 validator ──────────────────────────────────────────────

/// Maximum number of bytes that can be carried over between read chunks
/// (a UTF-8 code point is at most 4 bytes, so the carry buffer holds at
/// most 3 bytes from a split multi-byte sequence).
const UTF8_CARRY_BYTES: usize = 3;

/// Encoding name used when lossy UTF-8 conversion is applied.
const UTF8_LOSSY_ENCODING_NAME: &str = "UTF-8-lossy";

/// Status returned from [`Utf8StreamValidator::feed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Utf8StreamStatus {
    /// The combined bytes (carry + chunk) are valid UTF-8.
    Valid,
    /// The input contains an invalid UTF-8 byte sequence.
    Invalid,
    /// The last sequence in the input is incomplete; more bytes needed.
    Incomplete,
}

/// Streaming UTF-8 validator that preserves state across chunk boundaries.
///
/// Maintains a carry buffer of up to [`UTF8_CARRY_BYTES`] bytes so that
/// multi-byte UTF-8 characters split across chunk boundaries are correctly
/// handled.
#[derive(Default)]
struct Utf8StreamValidator {
    /// Buffer for incomplete multi-byte sequence carried over from the
    /// previous chunk.
    carry: [u8; UTF8_CARRY_BYTES],
    /// Number of valid bytes in [`carry`].
    carry_len: usize,
}

impl Utf8StreamValidator {
    /// Create a new streaming UTF-8 validator with no carry-over bytes.
    fn new() -> Self {
        Self::default()
    }

    /// Feed a byte chunk for validation.
    ///
    /// Returns `Utf8StreamStatus::Valid` if the chunk (including any
    /// carry-over from the previous chunk) is valid UTF-8. Returns
    /// `Incomplete` if the chunk ends with a partial multi-byte sequence,
    /// which is saved for the next call. Returns `Invalid` if any
    /// invalid byte sequence is found.
    ///
    /// When `is_eof` is `true`, an incomplete trailing sequence is treated
    /// as `Invalid`.
    fn feed(&mut self, chunk: &[u8], is_eof: bool) -> Utf8StreamStatus {
        let n_carry = self.carry_len;

        let total_len = n_carry + chunk.len();
        let mut combined = Vec::with_capacity(total_len);
        combined.extend_from_slice(&self.carry[..n_carry]);
        combined.extend_from_slice(chunk);

        self.carry_len = 0;

        let mut pos = 0;
        while pos < total_len {
            let b = combined[pos];
            let seq_len: usize = if b & 0x80 == 0 {
                1
            } else if b & 0xE0 == 0xC0 {
                2
            } else if b & 0xF0 == 0xE0 {
                3
            } else if b & 0xF8 == 0xF0 {
                4
            } else {
                // Continuation byte appearing as lead byte
                return Utf8StreamStatus::Invalid;
            };

            let remaining = total_len - pos;
            if remaining < seq_len {
                // Multi-byte sequence truncated at the end — save to carry
                self.carry_len = remaining;
                self.carry[..remaining].copy_from_slice(&combined[pos..]);
                if is_eof {
                    return Utf8StreamStatus::Invalid;
                }
                return Utf8StreamStatus::Incomplete;
            }

            // Validate continuation bytes
            for j in 1..seq_len {
                if combined[pos + j] & 0xC0 != 0x80 {
                    return Utf8StreamStatus::Invalid;
                }
            }

            // Reject overlong sequences
            if seq_len == 2 && b & 0xFE == 0xC0 {
                return Utf8StreamStatus::Invalid;
            }
            if seq_len == 3 && b == 0xE0 && combined[pos + 1] & 0xE0 == 0x80 {
                return Utf8StreamStatus::Invalid;
            }
            if seq_len == 4 && b == 0xF0 && combined[pos + 1] & 0xF0 == 0x80 {
                return Utf8StreamStatus::Invalid;
            }

            // Reject surrogate halves (U+D800..U+DFFF)
            if seq_len == 3 && b == 0xED && combined[pos + 1] & 0xE0 == 0xA0 {
                return Utf8StreamStatus::Invalid;
            }

            // Reject values > U+10FFFF
            if seq_len == 4 && (b > 0xF4 || (b == 0xF4 && combined[pos + 1] > 0x8F)) {
                return Utf8StreamStatus::Invalid;
            }

            pos += seq_len;
        }

        Utf8StreamStatus::Valid
    }
}

// ── Line ending detection ────────────────────────────────────────────────

// ── Non-UTF-8 decode ─────────────────────────────────────────────────────

/// Heuristic: when the input has at least 16 valid non-ASCII characters and
/// at most 4 invalid byte sequences, treat it as UTF-8 with minor corruption
/// and use lossy conversion ([`String::from_utf8_lossy`]). Otherwise fall
/// through to full charset detection / transcoding.
///
/// This avoids false positives for GB18030/Shift_JIS/etc. on valid UTF-8
/// Chinese text: the GB18030 decoder would silently "succeed" and produce
/// mojibake (e.g. "你好世界" → "浣犲ソ涓栫晫").
fn should_use_utf8_lossy(raw: &[u8]) -> bool {
    let mut valid_non_ascii = 0usize;
    let mut invalid_sequences = 0usize;
    let mut offset = 0usize;

    while offset < raw.len() {
        match std::str::from_utf8(&raw[offset..]) {
            Ok(valid) => {
                valid_non_ascii += valid.chars().filter(|ch| !ch.is_ascii()).count();
                break;
            }
            Err(error) => {
                let valid = &raw[offset..offset + error.valid_up_to()];
                if let Ok(valid_text) = std::str::from_utf8(valid) {
                    valid_non_ascii += valid_text.chars().filter(|ch| !ch.is_ascii()).count();
                }

                invalid_sequences += 1;
                let skip = error.error_len().unwrap_or(1);
                offset += error.valid_up_to() + skip;
            }
        }
    }

    valid_non_ascii >= 16 && invalid_sequences <= 4
}

/// Decode non-UTF-8 bytes into UTF-8.
///
/// Lossy-first strategy: check [`should_use_utf8_lossy`] first. If the input
/// is mostly-valid UTF-8 with minor corruption, use lossy conversion directly
/// to avoid GB18030/Shift_JIS/etc. producing mojibake on valid UTF-8
/// non-ASCII content. Otherwise fall through to full charset detection /
/// transcoding.
///
/// Returns `(utf8_bytes, encoding_name)`.
fn decode_non_utf8_bytes(raw: Vec<u8>) -> (Vec<u8>, &'static str) {
    if should_use_utf8_lossy(&raw) {
        return (String::from_utf8_lossy(&raw).into_owned().into_bytes(), UTF8_LOSSY_ENCODING_NAME);
    }
    transcode_to_utf8(&raw)
}

/// Load a file directly into a GapBuffer (zero-copy path).
///
/// Reads the file in chunks, writing directly into the GapBuffer's gap.
/// Memory peak ≈ file size + 64KB chunk buffer (essentially 1×).
///
/// UTF-8 validation uses [`Utf8StreamValidator`] so that multi-byte
/// characters split across chunk boundaries stay on the fast path.
///
/// Rejects binary files (null bytes in first 8KB).
pub fn load_file(path: &Path) -> Result<(GapBuffer, FileMetadata), FileError> {
    let mut file = fs::File::open(path).map_err(FileError::Io)?;
    let mut buf = GapBuffer::new(false).map_err(FileError::Io)?;

    // Read first 8KB for binary detection + BOM
    let mut first_chunk = [0u8; FIRST_CHUNK_SIZE];
    let n = file.read(&mut first_chunk).map_err(FileError::Io)?;

    // Binary detection: null byte in first 8KB
    if first_chunk[..n].contains(&0) {
        return Err(FileError::Binary);
    }

    // BOM detection
    let (had_bom, start) = strip_bom(&first_chunk[..n]);

    // Track line endings during loading
    let mut endings = LineEndingScan::default();
    let mut original_encoding: Option<&'static str> = None;

    // ── Streaming UTF-8 validation (first chunk) ────────────────────
    let first_is_eof = n < FIRST_CHUNK_SIZE;
    let mut validator = Utf8StreamValidator::new();
    let first_status = validator.feed(start, first_is_eof);

    match first_status {
        Utf8StreamStatus::Valid | Utf8StreamStatus::Incomplete => {
            if !start.is_empty() {
                endings.scan(start);
                let gap = buf.allocate_gap(0, start.len(), 0);
                gap[..start.len()].copy_from_slice(start);
                buf.commit_gap(start.len());
            }
        }
        Utf8StreamStatus::Invalid => {
            let mut all_bytes = Vec::with_capacity(start.len() + READ_CHUNK_SIZE);
            all_bytes.extend_from_slice(start);
            let mut read_buf = [0u8; READ_CHUNK_SIZE];
            loop {
                let n = file.read(&mut read_buf).map_err(FileError::Io)?;
                if n == 0 {
                    break;
                }
                all_bytes.extend_from_slice(&read_buf[..n]);
            }
            let (transcoded, encoding_name) = decode_non_utf8_bytes(all_bytes);
            original_encoding = Some(encoding_name);
            endings.scan(&transcoded);
            let gap = buf.allocate_gap(0, transcoded.len(), 0);
            gap[..transcoded.len()].copy_from_slice(&transcoded);
            buf.commit_gap(transcoded.len());
            return Ok((
                buf,
                FileMetadata { line_ending: endings.detected(), had_bom, original_encoding },
            ));
        }
    }

    // ── Streaming UTF-8 validation (remaining chunks) ───────────────
    let chunk_size: usize = READ_CHUNK_SIZE;
    loop {
        let offset = buf.len();
        let gap = buf.allocate_gap(offset, chunk_size, 0);
        let to_read = gap.len().min(chunk_size);
        let n = file.read(&mut gap[..to_read]).map_err(FileError::Io)?;
        if n == 0 {
            // EOF: any incomplete carry-over sequence is now truly invalid.
            let final_status = validator.feed(&[], true);
            if final_status == Utf8StreamStatus::Invalid {
                let raw = read_all(&buf);
                let (transcoded, encoding_name) = decode_non_utf8_bytes(raw);
                original_encoding = Some(encoding_name);
                buf = GapBuffer::new(false).map_err(FileError::Io)?;
                endings = LineEndingScan::default();
                endings.scan(&transcoded);
                let gap = buf.allocate_gap(0, transcoded.len(), 0);
                gap[..transcoded.len()].copy_from_slice(&transcoded);
                buf.commit_gap(transcoded.len());
            }
            buf.commit_gap(0);
            break;
        }

        let status = validator.feed(&gap[..n], false);
        match status {
            Utf8StreamStatus::Valid | Utf8StreamStatus::Incomplete => {
                endings.scan(&gap[..n]);
                buf.commit_gap(n);
            }
            Utf8StreamStatus::Invalid => {
                // Commit the failing chunk so read_all picks it up.
                buf.commit_gap(n);
                let mut all_bytes = read_all(&buf);
                let mut read_buf = [0u8; READ_CHUNK_SIZE];
                loop {
                    let r = file.read(&mut read_buf).map_err(FileError::Io)?;
                    if r == 0 {
                        break;
                    }
                    all_bytes.extend_from_slice(&read_buf[..r]);
                }
                let (transcoded, encoding_name) = decode_non_utf8_bytes(all_bytes);
                original_encoding = Some(encoding_name);
                buf = GapBuffer::new(false).map_err(FileError::Io)?;
                endings = LineEndingScan::default();
                endings.scan(&transcoded);
                let gap = buf.allocate_gap(0, transcoded.len(), 0);
                gap[..transcoded.len()].copy_from_slice(&transcoded);
                buf.commit_gap(transcoded.len());
                break;
            }
        }
    }

    let line_ending = endings.detected();
    Ok((buf, FileMetadata { line_ending, had_bom, original_encoding }))
}

#[derive(Default)]
struct LineEndingScan {
    has_lf: bool,
    has_cr: bool,
    has_crlf: bool,
    pending_cr: bool,
}

impl LineEndingScan {
    fn scan(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.pending_cr {
                self.pending_cr = false;
                if byte == b'\n' {
                    self.has_crlf = true;
                    continue;
                }
                self.has_cr = true;
            }
            match byte {
                b'\r' => self.pending_cr = true,
                b'\n' => self.has_lf = true,
                _ => {}
            }
        }
    }

    fn detected(&self) -> LineEnding {
        match (self.has_lf, self.has_crlf, self.has_cr || self.pending_cr) {
            (false, false, false) => LineEnding::None,
            (false, true, false) => LineEnding::Crlf,
            (true, false, false) => LineEnding::Lf,
            (false, false, true) => LineEnding::Cr,
            _ => LineEnding::Mixed,
        }
    }
}

/// Errors that can occur when saving a file.
#[derive(Debug)]
pub enum SaveError {
    Io {
        operation: &'static str,
        source: io::Error,
    },
    /// Target file exists and is read-only.
    ReadOnly,
    ConcurrentModification {
        expected: Option<Box<DiskRevision>>,
        actual: Option<Box<DiskRevision>>,
    },
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Io { operation, source } => {
                write!(f, "{operation} failed: {source}")
            }
            SaveError::ReadOnly => write!(f, "file is read-only"),
            SaveError::ConcurrentModification { .. } => {
                write!(f, "file changed during save")
            }
        }
    }
}

impl std::error::Error for SaveError {}

impl From<io::Error> for SaveError {
    fn from(e: io::Error) -> Self {
        SaveError::Io { operation: "save file", source: e }
    }
}

fn save_io(operation: &'static str, source: io::Error) -> SaveError {
    SaveError::Io { operation, source }
}

// ── Internal helpers ───────────────────────────────────────────────────────

/// Read all content from a GapBuffer into a contiguous Vec<u8>.
fn read_all(buffer: &GapBuffer) -> Vec<u8> {
    let total = buffer.len();
    let mut out = Vec::with_capacity(total);
    let mut off = 0;
    while off < total {
        let chunk = buffer.read_forward(off);
        if chunk.is_empty() {
            break;
        }
        let take = chunk.len().min(total - off);
        out.extend_from_slice(&chunk[..take]);
        off += take;
    }
    out
}

/// Normalize all line endings (CR, CRLF) to LF.
fn normalize_to_lf(content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len());
    let mut i = 0;
    while i < content.len() {
        if content[i] == b'\r' && i + 1 < content.len() && content[i + 1] == b'\n' {
            out.push(b'\n');
            i += 2;
        } else if content[i] == b'\r' {
            out.push(b'\n');
            i += 1;
        } else {
            out.push(content[i]);
            i += 1;
        }
    }
    out
}

/// Convert LF line endings to the target format.
fn convert_line_endings(content: &[u8], ending: LineEnding) -> Vec<u8> {
    match ending {
        LineEnding::Crlf => {
            let mut out = Vec::with_capacity(content.len() + content.len() / 50);
            for &b in content {
                if b == b'\n' {
                    out.push(b'\r');
                }
                out.push(b);
            }
            out
        }
        LineEnding::Cr => {
            let mut out = Vec::with_capacity(content.len());
            for &b in content {
                if b == b'\n' {
                    out.push(b'\r');
                } else {
                    out.push(b);
                }
            }
            out
        }
        _ => content.to_vec(),
    }
}

/// Serialize a snapshot with the requested line endings and optional UTF-8 BOM.
/// Mixed and unspecified endings preserve the original bytes.
pub fn serialized_file_contents(source: &[u8], ending: LineEnding, had_bom: bool) -> Vec<u8> {
    let contents = match ending {
        LineEnding::Mixed | LineEnding::None => source.to_vec(),
        _ => convert_line_endings(&normalize_to_lf(source), ending),
    };
    if !had_bom {
        return contents;
    }
    let mut encoded = Vec::with_capacity(contents.len() + UTF8_BOM.len());
    encoded.extend_from_slice(UTF8_BOM);
    encoded.extend_from_slice(&contents);
    encoded
}

/// Atomically save the buffer using the document's line endings and UTF-8 BOM.
pub fn save_file(
    buffer: &GapBuffer,
    path: &Path,
    metadata: &FileMetadata,
) -> Result<(), SaveError> {
    let raw = read_all(buffer);
    let contents = serialized_file_contents(&raw, metadata.line_ending, metadata.had_bom);
    save_atomically(path, &contents, SavePrecondition::Unconditional).map(|_| ())
}

const SAVE_TEMP_PREFIX: &str = ".textora-save-";
const SAVE_TEMP_ATTEMPTS: u64 = 100;
const RENAME_SAVE_OPERATION: &str = "rename saved file";
static SAVE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static SAVE_COMMIT_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy)]
enum SavePrecondition<'a> {
    Unconditional,
    Unchanged(Option<&'a DiskRevision>),
}

impl SavePrecondition<'_> {
    fn check(self, path: &Path) -> Result<(), SaveError> {
        let Self::Unchanged(expected) = self else { return Ok(()) };
        let actual = read_revision_for_save(path)?;
        if actual.as_ref() == expected {
            return Ok(());
        }
        Err(SaveError::ConcurrentModification {
            expected: expected.cloned().map(Box::new),
            actual: actual.map(Box::new),
        })
    }
}

fn create_save_temp_file(
    parent: &Path,
    replacing_existing: bool,
) -> Result<(std::path::PathBuf, fs::File), SaveError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if replacing_existing {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = replacing_existing;

    for _ in 0..SAVE_TEMP_ATTEMPTS {
        let counter = SAVE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let filename = format!("{SAVE_TEMP_PREFIX}{}-{counter}.tmp", std::process::id());
        let path = parent.join(filename);
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(save_io("create save temporary file", error)),
        }
    }
    Err(save_io(
        "create save temporary file",
        io::Error::new(io::ErrorKind::AlreadyExists, "temporary file name exhausted"),
    ))
}

fn writable_target_permissions(path: &Path) -> Result<Option<fs::Permissions>, SaveError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(save_io("read save target permissions", error)),
    };
    let permissions = metadata.permissions();
    if permissions.readonly() {
        return Err(SaveError::ReadOnly);
    }
    Ok(Some(permissions))
}

fn save_parent(path: &Path) -> &Path {
    path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."))
}

fn save_atomically(
    path: &Path,
    contents: &[u8],
    precondition: SavePrecondition<'_>,
) -> Result<Option<DiskRevision>, SaveError> {
    precondition.check(path)?;
    let existing = writable_target_permissions(path)?.is_some();
    let parent = save_parent(path);
    let save = || stage_and_commit_save(path, parent, contents, precondition, existing);
    match save() {
        Err(SaveError::Io { operation: RENAME_SAVE_OPERATION, source })
            if source.kind() == io::ErrorKind::CrossesDevices =>
        {
            // Resolve the destination directory again if its mount changed.
            // Stage a new file there so the destination remains atomic.
            save()
        }
        result => result,
    }
}

fn stage_and_commit_save(
    path: &Path,
    parent: &Path,
    contents: &[u8],
    precondition: SavePrecondition<'_>,
    replacing_existing: bool,
) -> Result<Option<DiskRevision>, SaveError> {
    let (temporary_path, mut file) = create_save_temp_file(parent, replacing_existing)?;
    let result = (|| {
        file.write_all(contents).map_err(|source| save_io("write save temporary file", source))?;

        // Compare and publish together for saves made by this process.
        let commit = SAVE_COMMIT_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        precondition.check(path)?;
        let permissions = writable_target_permissions(path)?;
        #[cfg(unix)]
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)
                .map_err(|source| save_io("preserve save target permissions", source))?;
        }
        #[cfg(not(unix))]
        let _ = permissions;
        file.sync_all().map_err(|source| save_io("sync save temporary file", source))?;
        let revision = match precondition {
            SavePrecondition::Unconditional => None,
            SavePrecondition::Unchanged(_) => {
                let metadata =
                    file.metadata().map_err(|source| save_io("read saved metadata", source))?;
                Some(revision_from_hash(path, &metadata, blake3::hash(contents)))
            }
        };
        drop(file);

        fs::rename(&temporary_path, path)
            .map_err(|source| save_io(RENAME_SAVE_OPERATION, source))?;
        drop(commit);
        sync_parent_directory(parent)?;
        Ok(revision)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary_path);
    }
    result
}

fn read_revision_for_save(path: &Path) -> Result<Option<DiskRevision>, SaveError> {
    read_disk_revision(path).map_err(|error| match error {
        crate::file::FileError::Io(source) => save_io("read disk revision", source),
        crate::file::FileError::Binary => save_io(
            "read disk revision",
            io::Error::new(io::ErrorKind::InvalidData, "binary file detected"),
        ),
    })
}

fn sync_parent_directory(parent: &Path) -> Result<(), SaveError> {
    #[cfg(unix)]
    {
        let directory =
            fs::File::open(parent).map_err(|source| save_io("open parent directory", source))?;
        directory.sync_all().map_err(|source| save_io("sync parent directory", source))?;
    }
    #[cfg(not(unix))]
    let _ = parent;
    Ok(())
}

/// Atomically save only if the destination still has the expected revision.
pub fn save_file_if_unchanged(
    path: &Path,
    contents: &[u8],
    expected: Option<&DiskRevision>,
) -> Result<DiskRevision, SaveError> {
    let revision = save_atomically(path, contents, SavePrecondition::Unchanged(expected))?;
    Ok(revision.expect("a revision-checked save returns the revision of its published bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk_revision::read_disk_revision;
    use crate::document::ReadableDocument;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn save_file_if_unchanged_writes_when_revision_matches() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        std::fs::write(&path, b"old").expect("file should be written");
        let expected = read_disk_revision(&path)
            .expect("revision should be readable")
            .expect("file should exist");

        let actual = save_file_if_unchanged(&path, b"new", Some(&expected))
            .expect("matching revision should allow save");

        assert_eq!(std::fs::read(&path).expect("saved file should be readable"), b"new");
        assert_ne!(actual.content_hash, expected.content_hash);
    }

    #[cfg(unix)]
    #[test]
    fn checked_save_preserves_mode_and_respects_readonly() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("save fixture directory");
        let path = directory.path().join("notes.md");
        fs::write(&path, b"old").expect("save fixture");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
            .expect("set writable fixture mode");
        let expected = read_disk_revision(&path).expect("read revision").expect("existing file");

        let saved =
            save_file_if_unchanged(&path, b"new", Some(&expected)).expect("save existing file");
        assert_eq!(
            fs::metadata(&path).expect("saved metadata").permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(read_disk_revision(&path).expect("saved revision"), Some(saved));

        fs::set_permissions(&path, fs::Permissions::from_mode(0o440))
            .expect("set readonly fixture mode");
        let expected = read_disk_revision(&path).expect("read revision").expect("existing file");
        assert!(matches!(
            save_file_if_unchanged(&path, b"newer", Some(&expected)),
            Err(SaveError::ReadOnly)
        ));
        assert_eq!(fs::read(&path).expect("read readonly fixture"), b"new");
    }

    #[test]
    fn load_crlf_split_across_stream_reads() {
        let directory = tempfile::tempdir().expect("line ending fixture directory");
        let path = directory.path().join("split-crlf.txt");
        for split in [FIRST_CHUNK_SIZE, FIRST_CHUNK_SIZE + READ_CHUNK_SIZE] {
            let mut contents = vec![b'a'; split - 1];
            contents.extend_from_slice(b"\r\nnext\r\n");
            fs::write(&path, &contents).expect("line ending fixture");
            let (buffer, metadata) = load_file(&path).expect("load line ending fixture");
            assert_eq!(metadata.line_ending, LineEnding::Crlf, "split at byte {split}");
            assert_eq!(read_all(&buffer), contents);
        }
    }

    #[test]
    fn save_file_if_unchanged_rejects_external_change_without_overwriting() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        std::fs::write(&path, b"old").expect("file should be written");
        let expected = read_disk_revision(&path)
            .expect("revision should be readable")
            .expect("file should exist");
        std::fs::write(&path, b"remote").expect("external change should be written");

        let result = save_file_if_unchanged(&path, b"local", Some(&expected));

        assert!(matches!(result, Err(SaveError::ConcurrentModification { .. })));
        assert_eq!(std::fs::read(&path).expect("file should be readable"), b"remote");
    }

    #[test]
    fn save_file_if_unchanged_does_not_recreate_deleted_target() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("notes.md");
        std::fs::write(&path, b"old").expect("file should be written");
        let expected = read_disk_revision(&path)
            .expect("revision should be readable")
            .expect("file should exist");
        std::fs::remove_file(&path).expect("target should be deleted");

        let result = save_file_if_unchanged(&path, b"local", Some(&expected));

        assert!(matches!(result, Err(SaveError::ConcurrentModification { .. })));
        assert!(!path.exists());
    }

    #[test]
    fn save_file_if_unchanged_creates_new_target_only_for_none_revision() {
        let directory = tempfile::tempdir().expect("temporary directory should exist");
        let path = directory.path().join("new.md");

        let revision = save_file_if_unchanged(&path, b"new", None)
            .expect("missing target with no baseline should be creatable");

        assert_eq!(std::fs::read(&path).expect("new file should be readable"), b"new");
        assert_eq!(revision.path, path);
        assert!(
            directory
                .path()
                .read_dir()
                .expect("directory should be readable")
                .filter_map(Result::ok)
                .all(|entry| entry.file_name() == "new.md")
        );
    }

    // ── Line ending detection ──────────────────────────────────────────────

    fn detect_ending(bytes: &[u8]) -> LineEnding {
        let mut endings = LineEndingScan::default();
        endings.scan(bytes);
        endings.detected()
    }

    #[test]
    fn detect_lf() {
        assert_eq!(detect_ending(b"hello\nworld\n"), LineEnding::Lf);
    }

    #[test]
    fn detect_crlf() {
        assert_eq!(detect_ending(b"hello\r\nworld\r\n"), LineEnding::Crlf);
    }

    #[test]
    fn detect_cr() {
        assert_eq!(detect_ending(b"hello\rworld\r"), LineEnding::Cr);
    }

    #[test]
    fn detect_mixed() {
        assert_eq!(detect_ending(b"hello\r\nworld\nfoo\r"), LineEnding::Mixed);
    }

    #[test]
    fn detect_none() {
        assert_eq!(detect_ending(b"hello world"), LineEnding::None);
    }

    #[test]
    fn detect_empty() {
        assert_eq!(detect_ending(b""), LineEnding::None);
    }

    // ── BOM handling ───────────────────────────────────────────────────────

    #[test]
    fn strip_utf8_bom() {
        let with_bom: &[u8] = b"\xEF\xBB\xBFhello";
        let (had, stripped) = strip_bom(with_bom);
        assert!(had);
        assert_eq!(stripped, b"hello");
    }

    #[test]
    fn no_bom() {
        let no_bom: &[u8] = b"hello";
        let (had, stripped) = strip_bom(no_bom);
        assert!(!had);
        assert_eq!(stripped, b"hello");
    }

    #[test]
    fn bom_only() {
        let bom_only: &[u8] = b"\xEF\xBB\xBF";
        let (had, stripped) = strip_bom(bom_only);
        assert!(had);
        assert_eq!(stripped, b"");
    }

    // ── load_file (zero-copy path) ─────────────────────────────────────────

    #[test]
    fn load_file_lf() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "hello\nworld\n").unwrap();
        let (gb, meta) = load_file(f.path()).unwrap();
        assert_eq!(meta.line_ending, LineEnding::Lf);
        assert!(!meta.had_bom);
        assert_eq!(gb.len(), 12); // "hello\nworld\n"
        let slice = gb.read_forward(0);
        assert!(slice.starts_with(b"hello"));
    }

    #[test]
    fn load_file_crlf() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "hello\r\nworld\r\n").unwrap();
        let (_, meta) = load_file(f.path()).unwrap();
        assert_eq!(meta.line_ending, LineEnding::Crlf);
    }

    #[test]
    fn load_file_with_bom() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"\xEF\xBB\xBFhello\n").unwrap();
        let (gb, meta) = load_file(f.path()).unwrap();
        assert!(meta.had_bom);
        // BOM should be stripped — content is "hello\n" (6 bytes)
        assert_eq!(gb.len(), 6);
        let slice = gb.read_forward(0);
        assert_eq!(slice, b"hello\n");
    }

    #[test]
    fn load_file_binary_rejected() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"hello\x00world\n").unwrap();
        assert!(matches!(load_file(f.path()), Err(FileError::Binary)));
    }

    #[test]
    fn load_file_empty() {
        let f = NamedTempFile::new().unwrap();
        let (gb, meta) = load_file(f.path()).unwrap();
        assert_eq!(gb.len(), 0);
        assert_eq!(meta.line_ending, LineEnding::None);
    }

    #[test]
    fn load_file_nonexistent() {
        assert!(matches!(
            load_file(Path::new("/tmp/nonexistent_textora_test.txt")),
            Err(FileError::Io(_))
        ));
    }

    #[test]
    fn load_file_multiline_content() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "line1").unwrap();
        writeln!(f, "line2").unwrap();
        writeln!(f, "line3").unwrap();
        let (gb, meta) = load_file(f.path()).unwrap();
        assert_eq!(meta.line_ending, LineEnding::Lf);
        // Read all content from GapBuffer
        let mut content = Vec::new();
        let mut off = 0;
        while off < gb.len() {
            let chunk = gb.read_forward(off);
            if chunk.is_empty() {
                break;
            }
            content.extend_from_slice(chunk);
            off += chunk.len();
        }
        let text = String::from_utf8(content).unwrap();
        assert_eq!(text, "line1\nline2\nline3\n");
    }

    // ── read_all ──────────────────────────────────────────────────────

    #[test]
    fn read_all_empty() {
        let gb = GapBuffer::new(false).unwrap();
        assert_eq!(read_all(&gb), b"");
    }

    #[test]
    fn read_all_simple() {
        let mut gb = GapBuffer::new(false).unwrap();
        let data = b"hello\nworld";
        let gap = gb.allocate_gap(0, data.len(), 0);
        gap[..data.len()].copy_from_slice(data);
        gb.commit_gap(data.len());
        assert_eq!(read_all(&gb), b"hello\nworld");
    }

    // ── normalize_to_lf ───────────────────────────────────────────────

    #[test]
    fn normalize_crlf_to_lf() {
        assert_eq!(normalize_to_lf(b"a\r\nb\r\n"), b"a\nb\n");
    }

    #[test]
    fn normalize_cr_to_lf() {
        assert_eq!(normalize_to_lf(b"a\rb\r"), b"a\nb\n");
    }

    #[test]
    fn normalize_mixed_to_lf() {
        assert_eq!(normalize_to_lf(b"a\r\nb\nc\r"), b"a\nb\nc\n");
    }

    #[test]
    fn normalize_pure_lf_unchanged() {
        assert_eq!(normalize_to_lf(b"a\nb"), b"a\nb");
    }

    // ── convert_line_endings ──────────────────────────────────────────

    #[test]
    fn convert_lf_to_lf() {
        assert_eq!(convert_line_endings(b"a\nb\n", LineEnding::Lf), b"a\nb\n");
    }

    #[test]
    fn convert_lf_to_crlf() {
        assert_eq!(convert_line_endings(b"a\nb\n", LineEnding::Crlf), b"a\r\nb\r\n");
    }

    #[test]
    fn convert_lf_to_cr() {
        assert_eq!(convert_line_endings(b"a\nb\n", LineEnding::Cr), b"a\rb\r");
    }

    #[test]
    fn convert_no_newlines() {
        assert_eq!(convert_line_endings(b"hello", LineEnding::Crlf), b"hello");
    }

    #[test]
    fn convert_none_ending() {
        assert_eq!(convert_line_endings(b"a\nb", LineEnding::None), b"a\nb");
    }

    #[test]
    fn staging_creates_distinct_files_in_target_directory() {
        let directory = tempfile::tempdir().expect("save fixture directory");
        let (first_path, _first) =
            create_save_temp_file(directory.path(), false).expect("first staging file");
        let (second_path, _second) =
            create_save_temp_file(directory.path(), false).expect("second staging file");
        assert_ne!(first_path, second_path);
        assert_eq!(first_path.parent(), Some(directory.path()));
        assert_eq!(second_path.parent(), Some(directory.path()));
        assert_eq!(save_parent(Path::new("notes.md")), Path::new("."));
    }

    // ── save_file ─────────────────────────────────────────────────────

    fn make_gap_buffer(content: &[u8]) -> GapBuffer {
        let mut gb = GapBuffer::new(false).unwrap();
        if !content.is_empty() {
            let gap = gb.allocate_gap(0, content.len(), 0);
            gap[..content.len()].copy_from_slice(content);
            gb.commit_gap(content.len());
        }
        gb
    }

    fn read_file_bytes(path: &Path) -> Vec<u8> {
        std::fs::read(path).unwrap()
    }

    #[test]
    fn save_preserves_lf() {
        let content = b"hello\nworld\n";
        let gb = make_gap_buffer(content);
        let meta =
            FileMetadata { line_ending: LineEnding::Lf, had_bom: false, original_encoding: None };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lf_test.txt");

        save_file(&gb, &path, &meta).unwrap();
        assert_eq!(read_file_bytes(&path), content);
        // No stray .tmp files visible (dir only has our file)
        let entries: Vec<_> =
            std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn save_preserves_crlf() {
        // Internal is LF, output should be CRLF
        let content_lf = b"hello\nworld\n";
        let gb = make_gap_buffer(content_lf);
        let meta =
            FileMetadata { line_ending: LineEnding::Crlf, had_bom: false, original_encoding: None };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crlf_test.txt");

        save_file(&gb, &path, &meta).unwrap();
        assert_eq!(read_file_bytes(&path), b"hello\r\nworld\r\n");
    }

    #[test]
    fn save_preserves_mixed_line_endings_without_editing() {
        let directory = tempfile::tempdir().expect("mixed line ending test directory must exist");
        let path = directory.path().join("mixed.md");
        let original = b"first\r\nsecond\nthird\r";
        let buffer = make_gap_buffer(original);
        let metadata = FileMetadata {
            line_ending: LineEnding::Mixed,
            had_bom: false,
            original_encoding: None,
        };
        save_file(&buffer, &path, &metadata).expect("mixed line ending fixture must save");
        assert_eq!(std::fs::read(path).expect("saved fixture must be readable"), original);
    }

    #[test]
    fn save_preserves_bom() {
        let content = b"hello\n";
        let gb = make_gap_buffer(content);
        let meta =
            FileMetadata { line_ending: LineEnding::Lf, had_bom: true, original_encoding: None };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bom_test.txt");

        save_file(&gb, &path, &meta).unwrap();
        let actual = read_file_bytes(&path);
        assert!(actual.starts_with(b"\xEF\xBB\xBF"), "BOM should be present");
        assert_eq!(&actual[3..], b"hello\n");
    }

    #[test]
    fn save_atomic_no_temp_leftover() {
        let gb = make_gap_buffer(b"data");
        let meta =
            FileMetadata { line_ending: LineEnding::Lf, had_bom: false, original_encoding: None };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atomic_test.txt");

        save_file(&gb, &path, &meta).unwrap();

        // No .tmp or .bak files left behind
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries.len(), 1, "only the target file should exist");
        assert_eq!(entries[0], "atomic_test.txt");
    }

    #[test]
    fn save_atomic_overwrite_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("overwrite_test.txt");

        // Write initial content
        std::fs::write(&path, b"original\n").unwrap();

        // Overwrite via save
        let gb = make_gap_buffer(b"new\ncontent\n");
        let meta =
            FileMetadata { line_ending: LineEnding::Lf, had_bom: false, original_encoding: None };
        save_file(&gb, &path, &meta).unwrap();

        assert_eq!(read_file_bytes(&path), b"new\ncontent\n");
    }

    #[cfg(unix)]
    #[test]
    fn save_keeps_file_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mode_test.txt");

        // Create file with specific mode
        {
            let f = std::fs::File::create(&path).unwrap();
            f.set_permissions(std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        // Write initial content so the file already exists
        std::fs::write(&path, b"initial\n").unwrap();
        {
            let f = std::fs::File::open(&path).unwrap();
            f.set_permissions(std::fs::Permissions::from_mode(0o640)).unwrap();
        }

        let gb = make_gap_buffer(b"new\n");
        let meta =
            FileMetadata { line_ending: LineEnding::Lf, had_bom: false, original_encoding: None };
        save_file(&gb, &path, &meta).unwrap();

        let actual_mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(
            actual_mode & 0o777,
            0o640,
            "file mode should be preserved: expected 0o640, got 0o{:o}",
            actual_mode
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_readonly_target_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("readonly_test.txt");
        std::fs::write(&path, b"cannot touch\n").unwrap();

        // Make read-only
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).unwrap();

        let gb = make_gap_buffer(b"new content");
        let meta =
            FileMetadata { line_ending: LineEnding::Lf, had_bom: false, original_encoding: None };
        let result = save_file(&gb, &path, &meta);
        assert!(
            matches!(result, Err(SaveError::ReadOnly)),
            "expected ReadOnly error, got {result:?}"
        );
    }

    #[test]
    fn save_to_nonexistent_directory() {
        let gb = make_gap_buffer(b"data");
        let meta =
            FileMetadata { line_ending: LineEnding::Lf, had_bom: false, original_encoding: None };
        let path = Path::new("/tmp/nonexistent_dir_xyz_editplus/file.txt");
        let result = save_file(&gb, path, &meta);
        assert!(result.is_err(), "save to nonexistent dir should fail");
    }

    #[test]
    fn save_empty_file() {
        let gb = make_gap_buffer(b"");
        let meta =
            FileMetadata { line_ending: LineEnding::None, had_bom: false, original_encoding: None };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");

        save_file(&gb, &path, &meta).unwrap();
        assert_eq!(read_file_bytes(&path), b"");
    }

    #[test]
    fn save_empty_file_with_bom() {
        let gb = make_gap_buffer(b"");
        let meta =
            FileMetadata { line_ending: LineEnding::None, had_bom: true, original_encoding: None };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty_bom.txt");

        save_file(&gb, &path, &meta).unwrap();
        // Even empty file should get BOM if metadata says so
        assert_eq!(read_file_bytes(&path), b"\xEF\xBB\xBF");
    }

    #[test]
    fn save_roundtrip_lf_content_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roundtrip.txt");

        let original = b"line1\nline2\nline3\n";
        std::fs::write(&path, original).unwrap();

        // Load
        let (gb, meta) = load_file(&path).unwrap();
        assert_eq!(meta.line_ending, LineEnding::Lf);

        // Save
        save_file(&gb, &path, &meta).unwrap();

        // Re-load and compare
        let (gb2, meta2) = load_file(&path).unwrap();
        assert_eq!(meta2.line_ending, LineEnding::Lf);
        assert_eq!(read_all(&gb), read_all(&gb2));
    }

    #[test]
    fn save_roundtrip_crlf_content_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roundtrip_crlf.txt");

        let original = b"line1\r\nline2\r\nline3\r\n";
        std::fs::write(&path, original).unwrap();

        let (gb, meta) = load_file(&path).unwrap();
        assert_eq!(meta.line_ending, LineEnding::Crlf);

        save_file(&gb, &path, &meta).unwrap();

        let (gb2, meta2) = load_file(&path).unwrap();
        assert_eq!(meta2.line_ending, LineEnding::Crlf);
        assert_eq!(read_all(&gb), read_all(&gb2));
    }

    #[test]
    fn save_roundtrip_bom_content_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("roundtrip_bom.txt");

        let with_bom: Vec<u8> = [b"\xEF\xBB\xBF".as_ref(), b"hello\nworld\n"].concat();
        std::fs::write(&path, &with_bom).unwrap();

        let (gb, meta) = load_file(&path).unwrap();
        assert!(meta.had_bom);
        assert_eq!(meta.line_ending, LineEnding::Lf);

        save_file(&gb, &path, &meta).unwrap();

        let (gb2, meta2) = load_file(&path).unwrap();
        assert!(meta2.had_bom);
        assert_eq!(read_all(&gb), read_all(&gb2));
    }

    // ── 编码检测与转码 ─────────────────────────────────────────────────

    #[test]
    fn load_gbk_file_transcodes_to_utf8() {
        // GBK 编码的 "你好\n" (0xC4E3 0xBAC3)
        let gbk_bytes: &[u8] = &[0xC4, 0xE3, 0xBA, 0xC3, 0x0A];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gbk_test.txt");
        std::fs::write(&path, gbk_bytes).unwrap();

        let (gb, meta) = load_file(&path).unwrap();
        assert_eq!(meta.original_encoding, Some("GBK"));
        let content = read_all(&gb);
        let text = std::str::from_utf8(&content).expect("transcoded content should be valid UTF-8");
        assert_eq!(text, "你好\n");
    }

    #[test]
    fn load_gb18030_four_byte_character_keeps_extended_encoding() {
        let directory = tempfile::tempdir().expect("encoding fixture directory");
        let path = directory.path().join("gb18030.txt");
        let (encoded, _, had_errors) = encoding_rs::GB18030.encode("你好😀\n");
        assert!(!had_errors, "fixture must be representable in GB18030");
        fs::write(&path, encoded).expect("write encoding fixture");

        let (buffer, metadata) = load_file(&path).expect("load encoding fixture");
        assert_eq!(metadata.original_encoding, Some("GB18030"));
        assert_eq!(read_all(&buffer), "你好😀\n".as_bytes());
    }

    #[test]
    fn load_utf8_file_skips_detection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("utf8_skip.txt");
        std::fs::write(&path, b"hello\n").unwrap();

        let (_, meta) = load_file(&path).unwrap();
        assert_eq!(meta.original_encoding, None, "UTF-8 file should not trigger transcoding");
    }

    #[test]
    fn load_shift_jis_file_transcodes() {
        // Shift_JIS 0x82A0 = あ (U+3042)
        let sjis_bytes: &[u8] = &[0x82, 0xA0, 0x0A];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sjis_test.txt");
        std::fs::write(&path, sjis_bytes).unwrap();

        let (gb, meta) = load_file(&path).unwrap();
        assert!(meta.original_encoding.is_some(), "should detect non-UTF-8 encoding");
        let content = read_all(&gb);
        let text = std::str::from_utf8(&content).expect("transcoded content should be valid UTF-8");
        assert!(!text.is_empty(), "transcoded text should not be empty");
    }
    #[test]
    fn load_midway_gbk_triggers_transcode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("midway_gbk.txt");

        // Build a file: 64KB of ASCII + GBK "你好\n"
        // First chunk (8KB) will pass is_valid_utf8, but later bytes will fail.
        let mut content = Vec::with_capacity(70000);
        content.extend_from_slice(&vec![b'A'; 65536]);
        content.extend_from_slice(&[0xC4, 0xE3, 0xBA, 0xC3, 0x0A]);
        std::fs::write(&path, &content).unwrap();

        let (gb, meta) = load_file(&path).unwrap();
        assert!(meta.original_encoding.is_some(), "midway GBK should trigger transcode");

        let result = read_all(&gb);
        let text = std::str::from_utf8(&result).expect("should be valid UTF-8 after transcode");
        assert!(text.contains('你'), "should contain transcoded Chinese character");
        assert!(text.contains('A'), "should contain the ASCII prefix");
    }

    #[test]
    fn load_large_valid_utf8_no_transcode() {
        // 65536 ASCII + 3-byte UTF-8 U+4F60 + LF = 65540 bytes.
        // Second chunk (offset 8192..73728) starts mid-ASCII, passes per-chunk UTF-8 check.
        // Final chunk (offset 65536..) starts with 0xE4 (valid 3-byte lead), also passes.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large_utf8_valid.txt");

        let mut content = Vec::with_capacity(70000);
        content.extend_from_slice(&vec![b'A'; 65536]);
        content.extend_from_slice(&[0xE4, 0xBD, 0xA0, 0x0A]); // U+4F60 + LF
        std::fs::write(&path, &content).unwrap();

        let (gb, meta) = load_file(&path).unwrap();
        assert_eq!(meta.original_encoding, None, "valid UTF-8 must not trigger transcode");
        let result = read_all(&gb);
        assert_eq!(result, content, "content must round-trip unchanged");
    }

    #[test]
    fn load_midway_invalid_byte_triggers_transcode() {
        // First 8KB passes is_valid_utf8, but a lone continuation byte (0x80) at
        // offset 73728 (chunk boundary) triggers the midway fallback.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("midway_invalid.txt");

        let mut content = vec![b'A'; 73728]; // ASCII prefix spanning multiple chunks
        content.push(0x80); // invalid standalone continuation byte
        std::fs::write(&path, &content).unwrap();

        let (gb, meta) = load_file(&path).unwrap();
        assert!(meta.original_encoding.is_some(), "invalid byte must trigger transcode");
        let result = read_all(&gb);
        assert!(std::str::from_utf8(&result).is_ok(), "transcoded content must be valid UTF-8");
    }

    #[test]
    fn load_large_cjk_utf8_first_chunk_boundary_no_transcode() {
        let dir = tempfile::tempdir().expect("create tempdir for CJK UTF-8 test");
        let path = dir.path().join("large_cjk_utf8_first_chunk.txt");

        let text = "你好世界".repeat(3000);
        let content = text.into_bytes();
        assert!(content.len() > FIRST_CHUNK_SIZE);
        assert!(
            std::str::from_utf8(&content[..FIRST_CHUNK_SIZE]).is_err(),
            "test fixture must split a UTF-8 character at the first chunk boundary"
        );

        std::fs::write(&path, &content).expect("write CJK UTF-8 fixture");

        let (gb, meta) = load_file(&path).expect("load valid CJK UTF-8 file");
        assert_eq!(meta.original_encoding, None, "valid UTF-8 must not transcode");
        assert_eq!(read_all(&gb), content, "valid UTF-8 bytes must round-trip unchanged");
    }

    #[test]
    fn load_large_cjk_utf8_many_chunk_boundaries_no_transcode() {
        let dir = tempfile::tempdir().expect("create tempdir for large CJK UTF-8 test");
        let path = dir.path().join("large_cjk_utf8_many_chunks.txt");

        let text = "这是连续中文内容，用来覆盖多个读取块边界。".repeat(8000);
        let content = text.into_bytes();
        assert!(content.len() > READ_CHUNK_SIZE * 2);

        std::fs::write(&path, &content).expect("write large CJK UTF-8 fixture");

        let (gb, meta) = load_file(&path).expect("load large valid CJK UTF-8 file");
        assert_eq!(meta.original_encoding, None, "valid UTF-8 must stay on fast path");
        assert_eq!(read_all(&gb), content, "large CJK UTF-8 content must remain unchanged");
    }

    #[test]
    fn load_mostly_utf8_with_sparse_invalid_byte_uses_utf8_lossy() {
        let dir = tempfile::tempdir().expect("create tempdir for mostly UTF-8 test");
        let path = dir.path().join("mostly_utf8_dirty.txt");

        let mut content = "你好，世界。这里是 UTF-8 小说正文。\n".repeat(32).into_bytes();
        content.push(0x80);
        std::fs::write(&path, &content).expect("write mostly UTF-8 fixture");

        let (gb, meta) = load_file(&path).expect("load mostly UTF-8 fixture");
        assert_eq!(meta.original_encoding, Some(UTF8_LOSSY_ENCODING_NAME));

        let loaded = read_all(&gb);
        let text = std::str::from_utf8(&loaded).expect("lossy UTF-8 output must be valid UTF-8");
        assert!(text.starts_with("你好，世界。"));
        assert!(text.contains('\u{FFFD}'), "invalid byte should become replacement character");
        assert!(
            !text.starts_with("浣犲ソ"),
            "mostly UTF-8 content must not be decoded as GB18030 mojibake"
        );
    }

    // ── Streaming UTF-8 validator ──────────────────────────────────────

    #[test]
    fn utf8_stream_validator_accepts_split_multibyte_character() {
        let mut validator = Utf8StreamValidator::new();

        let bytes = "你".as_bytes();

        assert_eq!(validator.feed(&bytes[..2], false), Utf8StreamStatus::Incomplete);
        assert_eq!(validator.feed(&bytes[2..], true), Utf8StreamStatus::Valid);
    }

    #[test]
    fn utf8_stream_validator_rejects_incomplete_sequence_at_eof() {
        let mut validator = Utf8StreamValidator::new();

        let bytes = "你".as_bytes();

        assert_eq!(validator.feed(&bytes[..2], true), Utf8StreamStatus::Invalid);
    }

    #[test]
    fn utf8_stream_validator_rejects_invalid_continuation_byte() {
        let mut validator = Utf8StreamValidator::new();

        assert_eq!(validator.feed(&[0xE4, 0x41, 0x80], true), Utf8StreamStatus::Invalid);
    }
}
