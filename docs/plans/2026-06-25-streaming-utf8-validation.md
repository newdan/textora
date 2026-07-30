# 流式 UTF-8 校验与编码转码修复方案

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复大 TXT 加载时因 chunk 边界截断 UTF-8 字符而误触发 GB18030/GBK 转码，导致首屏及全文乱码、加载显著变慢的问题。

**Architecture:** 文件加载先走流式 UTF-8 校验，校验器在 chunk 之间保留最多 3 字节未完成序列，确保合法 UTF-8 永不进入启发式转码路径。只有确认整流存在真实非法 UTF-8 字节后，才进入非 UTF-8 慢路径；慢路径再区分 BOM、CJK 编码和“基本是 UTF-8 但夹少量坏字节”的降级策略，避免把整篇 UTF-8 中文按 GB18030 重解码。

**Tech Stack:** Rust 1.93, `encoding_rs`, `GapBuffer`, `cargo test`, Criterion benchmarks.

## Global Constraints

- 全程遵守 `AGENTS.md`：中文沟通、先复现测试再修、严禁防御性叠补丁。
- `ui` 层不得依赖 `app` 状态；本方案只改 `crates/core`，不触碰跨层边界。
- Rust 状态表达优先使用 `enum`，避免多个 `bool` 组合表达互斥状态。
- 禁止滥用 `.unwrap()`；测试代码中如需简化，使用 `.expect("说明失败原因")`。
- 提交前必须 `cargo fmt`，并至少跑完本计划列出的 core/app 相关测试。
- 若实现阶段修改超过 3 个文件，必须拆成子任务执行并逐步验证。

---

## Problem Summary

当前 `crates/core/src/file.rs` 在 `load_file()` 中先读取固定 8192 字节作为首块，然后直接调用 `std::str::from_utf8(start)`。对于连续中文 UTF-8 文本，8192 字节很容易落在一个 3 字节汉字中间，首块本身会报 `unexpected end of data`，但整篇文件其实是合法 UTF-8。

误判后，当前实现会读取整文件并按候选编码顺序转码。候选列表优先尝试 GB18030，而 UTF-8 中文字节用 GB18030 解码会形成典型 mojibake，例如：

```text
原文: 你好世界
误解码: 浣犲ソ涓栫晫
```

这同时解释两个用户现象：

- 加载慢：合法 UTF-8 大文件误入整文件读取 + 整文件转码慢路径。
- 首屏乱码：首屏 UTF-8 中文被按 GB18030 解释。

已有最新修复只处理了后续 64KB chunk 的边界误判，没有覆盖首个 8KB chunk。

## Target Behavior

1. 合法 UTF-8 文件，无论字符是否跨 8KB/64KB 边界，都必须保持 `original_encoding == None`，内容字节原样进入 `GapBuffer`。
2. 以 ASCII 开头、后续出现 GBK/GB18030 字节的文件，仍然要转码为 UTF-8，并记录 `original_encoding`。
3. UTF-16 LE/BE BOM 文件继续优先按 BOM 解码。
4. 基本是 UTF-8、只夹少量非法字节的文件，不得整篇按 GB18030 重解码；应使用 UTF-8 lossy 方式保留绝大多数原文，并将坏字节替换为 `U+FFFD`。
5. 对合法 UTF-8 大文件，内存峰值保持接近当前零拷贝路径，不允许为了校验而整文件读入临时 `Vec`。

## File Structure

- Modify: `crates/core/src/file.rs`
  - 新增流式 UTF-8 校验类型。
  - 重构 `load_file()` 的编码分流。
  - 增加回归测试。
- Modify: `crates/core/benches/file_loading.rs`
  - 增加连续 CJK 大文件基准，覆盖首块和多 chunk 边界。
- Optional Modify: `docs/plans/2026-06-24-encoding-detection.md`
  - 实现完成后追加一段“后续修正”，说明旧方案的首块误判问题已被本方案替代。

## Design

### Encoding Decision Flow

```text
open file
  -> read first chunk for binary/BOM
  -> if UTF-16 BOM: read all and transcode by BOM
  -> otherwise stream file through UTF-8 validator
      -> ValidUtf8: commit chunks to GapBuffer without transcoding
      -> InvalidUtf8:
          -> collect raw bytes
          -> if mostly UTF-8 with sparse invalid bytes: UTF-8 lossy
          -> else transcode_to_utf8(raw) using legacy candidate list
```

### Core Types

Add these near the current `is_valid_utf8()` helper in `crates/core/src/file.rs`:

```rust
const FIRST_CHUNK_SIZE: usize = 8 * 1024;
const READ_CHUNK_SIZE: usize = 64 * 1024;
const UTF8_MAX_SEQUENCE_BYTES: usize = 4;
const UTF8_CARRY_BYTES: usize = UTF8_MAX_SEQUENCE_BYTES - 1;
const UTF8_LOSSY_ENCODING_NAME: &str = "UTF-8-lossy";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Utf8StreamStatus {
    Valid,
    Incomplete,
    Invalid,
}

#[derive(Debug, Default)]
struct Utf8StreamValidator {
    carry: [u8; UTF8_CARRY_BYTES],
    carry_len: usize,
}
```

The validator must have exactly one responsibility: decide whether bytes are valid UTF-8 across chunk boundaries. It must not know about `GapBuffer`, line endings, file metadata, or encoding candidates.

### Validator Interface

Use this interface so `load_file()` can keep IO and buffer writes separate:

```rust
impl Utf8StreamValidator {
    fn new() -> Self {
        Self::default()
    }

    fn feed(&mut self, bytes: &[u8], is_eof: bool) -> Utf8StreamStatus {
        if bytes.is_empty() {
            return self.finish(is_eof);
        }

        if self.carry_len == 0 {
            return self.validate_chunk(bytes, is_eof);
        }

        let mut combined = Vec::with_capacity(self.carry_len + bytes.len());
        combined.extend_from_slice(&self.carry[..self.carry_len]);
        combined.extend_from_slice(bytes);
        self.carry_len = 0;
        self.validate_chunk(&combined, is_eof)
    }

    fn finish(&self, is_eof: bool) -> Utf8StreamStatus {
        match (self.carry_len, is_eof) {
            (0, _) => Utf8StreamStatus::Valid,
            (_, true) => Utf8StreamStatus::Invalid,
            (_, false) => Utf8StreamStatus::Incomplete,
        }
    }

    fn validate_chunk(&mut self, bytes: &[u8], is_eof: bool) -> Utf8StreamStatus {
        match std::str::from_utf8(bytes) {
            Ok(_) => Utf8StreamStatus::Valid,
            Err(error) if error.error_len().is_none() && !is_eof => {
                let valid_up_to = error.valid_up_to();
                let suffix = &bytes[valid_up_to..];
                if suffix.len() > UTF8_CARRY_BYTES {
                    return Utf8StreamStatus::Invalid;
                }
                self.carry[..suffix.len()].copy_from_slice(suffix);
                self.carry_len = suffix.len();
                Utf8StreamStatus::Incomplete
            }
            Err(_) => Utf8StreamStatus::Invalid,
        }
    }
}
```

Implementation note: this simple version allocates only when a previous chunk ended with an incomplete UTF-8 sequence. That case is rare for ASCII and bounded for CJK. If profiling shows the allocation matters, replace the temporary `Vec` with a fixed scratch buffer plus separate validation of the carried prefix.

### Loader Refactor Shape

Replace ad hoc per-chunk `is_valid_utf8()` checks with a single decision helper:

```rust
enum LoadedContent {
    Utf8(GapBuffer),
    Transcoded { bytes: Vec<u8>, original_encoding: &'static str },
}

fn decode_non_utf8_bytes(raw: Vec<u8>) -> (Vec<u8>, &'static str) {
    if should_use_utf8_lossy(&raw) {
        return (
            String::from_utf8_lossy(&raw).into_owned().into_bytes(),
            UTF8_LOSSY_ENCODING_NAME,
        );
    }

    transcode_to_utf8(&raw)
}
```

`LoadedContent::Utf8` is for the common path and preserves zero-copy writes into `GapBuffer`. `LoadedContent::Transcoded` is only used after a real invalid UTF-8 byte is observed or BOM forces decoding.

### UTF-8 Lossy Heuristic

Add a narrow heuristic before legacy candidate decoding:

```rust
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
```

This intentionally handles a narrow case: “mostly valid UTF-8 text with a few bad bytes.” True GBK/GB18030 files typically do not contain long runs of valid UTF-8 Chinese characters, so they continue to the existing candidate decoder.

## Task 1: Regression Tests for Valid UTF-8 Boundaries

**Files:**
- Modify: `crates/core/src/file.rs`

**Interfaces:**
- Consumes: existing `load_file(path)` and test-only `read_all(&GapBuffer)`.
- Produces: failing tests that capture the large UTF-8 TXT regression.

- [ ] **Step 1: Add first-chunk CJK boundary regression**

Add this test in the existing `#[cfg(test)] mod tests` encoding section:

```rust
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
```

- [ ] **Step 2: Add multi-chunk CJK boundary regression**

```rust
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
```

- [ ] **Step 3: Run tests and verify they fail before implementation**

Run:

```bash
cargo test -p edit-plus-core load_large_cjk_utf8_first_chunk_boundary_no_transcode -- --nocapture
cargo test -p edit-plus-core load_large_cjk_utf8_many_chunk_boundaries_no_transcode -- --nocapture
```

Expected before implementation:

```text
FAILED
valid UTF-8 must not transcode
```

The first test is the critical reproduction for the user report.

## Task 2: Add Streaming UTF-8 Validator

**Files:**
- Modify: `crates/core/src/file.rs`

**Interfaces:**
- Consumes: byte chunks from `load_file()`.
- Produces: `Utf8StreamValidator::feed(bytes, is_eof) -> Utf8StreamStatus`.

- [ ] **Step 1: Add constants and validator types**

Insert the constants and types from “Core Types” above near `is_valid_utf8()`.

- [ ] **Step 2: Add validator implementation**

Insert the `impl Utf8StreamValidator` block from “Validator Interface” above.

- [ ] **Step 3: Add unit tests for validator only**

```rust
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
```

- [ ] **Step 4: Run validator tests**

Run:

```bash
cargo test -p edit-plus-core utf8_stream_validator -- --nocapture
```

Expected after this task:

```text
test result: ok. 3 passed
```

## Task 3: Refactor `load_file()` to Use the Validator

**Files:**
- Modify: `crates/core/src/file.rs`

**Interfaces:**
- Consumes: `Utf8StreamValidator`, `decode_non_utf8_bytes(raw)`.
- Produces: `load_file()` behavior matching Target Behavior.

- [ ] **Step 1: Replace magic chunk sizes**

Change:

```rust
let mut first_chunk = [0u8; 8192];
let chunk_size: usize = 65536;
```

To:

```rust
let mut first_chunk = [0u8; FIRST_CHUNK_SIZE];
let chunk_size: usize = READ_CHUNK_SIZE;
```

- [ ] **Step 2: Add non-UTF-8 decode helper**

Add:

```rust
fn decode_non_utf8_bytes(raw: Vec<u8>) -> (Vec<u8>, &'static str) {
    if should_use_utf8_lossy(&raw) {
        return (
            String::from_utf8_lossy(&raw).into_owned().into_bytes(),
            UTF8_LOSSY_ENCODING_NAME,
        );
    }

    transcode_to_utf8(&raw)
}
```

Also add `should_use_utf8_lossy()` from “UTF-8 Lossy Heuristic”.

- [ ] **Step 3: Change first-chunk handling**

Do not call `is_valid_utf8(start)` to decide immediate transcoding. Feed `start` into `Utf8StreamValidator` with `is_eof = n < FIRST_CHUNK_SIZE`.

Expected structure:

```rust
let first_is_eof = n < FIRST_CHUNK_SIZE;
let mut validator = Utf8StreamValidator::new();
let first_status = validator.feed(start, first_is_eof);

match first_status {
    Utf8StreamStatus::Valid | Utf8StreamStatus::Incomplete => {
        if !start.is_empty() {
            scan_line_endings(start, &mut has_lf, &mut has_cr, &mut has_crlf);
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
            let read_len = file.read(&mut read_buf).map_err(FileError::Io)?;
            if read_len == 0 {
                break;
            }
            all_bytes.extend_from_slice(&read_buf[..read_len]);
        }
        let (transcoded, encoding_name) = decode_non_utf8_bytes(all_bytes);
        original_encoding = Some(encoding_name);
        scan_line_endings(&transcoded, &mut has_lf, &mut has_cr, &mut has_crlf);
        let gap = buf.allocate_gap(0, transcoded.len(), 0);
        gap[..transcoded.len()].copy_from_slice(&transcoded);
        buf.commit_gap(transcoded.len());
        return Ok((buf, FileMetadata { line_ending: detect_line_ending(has_lf, has_crlf, has_cr), had_bom, original_encoding }));
    }
}
```

Before implementing this exact snippet, extract the existing line-ending match into:

```rust
fn detect_line_ending(has_lf: bool, has_crlf: bool, has_cr: bool) -> LineEnding {
    match (has_lf, has_crlf, has_cr) {
        (false, false, false) => LineEnding::None,
        (false, true, false) => LineEnding::Crlf,
        (true, false, false) => LineEnding::Lf,
        (false, false, true) => LineEnding::Cr,
        _ => LineEnding::Mixed,
    }
}
```

- [ ] **Step 4: Change loop handling**

For each later chunk, feed the bytes to the same validator. If status is valid or incomplete, commit bytes as UTF-8 fast path. If invalid, collect already committed bytes with `read_all(&buf)`, append current bytes and the remaining file, then call `decode_non_utf8_bytes()`.

Important invariant: a status of `Incomplete` is not an error unless it remains incomplete at EOF.

Use this EOF check when `file.read()` returns `0`:

```rust
let final_status = validator.feed(&[], true);
if final_status == Utf8StreamStatus::Invalid {
    let raw = read_all(&buf);
    let (transcoded, encoding_name) = decode_non_utf8_bytes(raw);
    original_encoding = Some(encoding_name);
    buf = GapBuffer::new(false).map_err(FileError::Io)?;
    has_lf = false;
    has_cr = false;
    has_crlf = false;
    scan_line_endings(&transcoded, &mut has_lf, &mut has_cr, &mut has_crlf);
    let gap = buf.allocate_gap(0, transcoded.len(), 0);
    gap[..transcoded.len()].copy_from_slice(&transcoded);
    buf.commit_gap(transcoded.len());
}
break;
```

- [ ] **Step 5: Run boundary tests**

Run:

```bash
cargo test -p edit-plus-core load_large_cjk_utf8_first_chunk_boundary_no_transcode -- --nocapture
cargo test -p edit-plus-core load_large_cjk_utf8_many_chunk_boundaries_no_transcode -- --nocapture
```

Expected:

```text
test result: ok. 1 passed
test result: ok. 1 passed
```

## Task 4: Preserve Existing Non-UTF-8 Behavior and Add Lossy Guardrail

**Files:**
- Modify: `crates/core/src/file.rs`

**Interfaces:**
- Consumes: `decode_non_utf8_bytes(raw)`.
- Produces: regression coverage for GBK and mostly-UTF-8 dirty files.

- [ ] **Step 1: Keep GBK midway test passing**

Run:

```bash
cargo test -p edit-plus-core load_midway_gbk_triggers_transcode -- --nocapture
```

Expected:

```text
test result: ok. 1 passed
```

- [ ] **Step 2: Add mostly-UTF-8 dirty-byte regression**

```rust
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
```

- [ ] **Step 3: Run guardrail tests**

Run:

```bash
cargo test -p edit-plus-core load_mostly_utf8_with_sparse_invalid_byte_uses_utf8_lossy -- --nocapture
cargo test -p edit-plus-core load_gbk_file_transcodes_to_utf8 -- --nocapture
cargo test -p edit-plus-core load_shift_jis_file_transcodes -- --nocapture
```

Expected:

```text
test result: ok. 1 passed
test result: ok. 1 passed
test result: ok. 1 passed
```

## Task 5: Benchmark Coverage for Large CJK TXT

**Files:**
- Modify: `crates/core/benches/file_loading.rs`

**Interfaces:**
- Consumes: `core::file::load_file`.
- Produces: performance visibility for the exact regression class.

- [ ] **Step 1: Add a boundary-heavy CJK generator**

```rust
fn generate_dense_cjk_file(path: &std::path::Path, size_mb: usize) {
    let line = "你好世界春夏秋冬山河湖海天地玄黄宇宙洪荒\n";
    let target = size_mb * 1024 * 1024;
    let mut file = std::fs::File::create(path).expect("create dense CJK benchmark file");
    let mut written = 0;

    while written < target {
        let bytes = line.as_bytes();
        let remaining = target - written;
        if remaining >= bytes.len() {
            file.write_all(bytes).expect("write dense CJK benchmark line");
            written += bytes.len();
        } else {
            file.write_all(&bytes[..remaining]).expect("write dense CJK benchmark tail");
            written += remaining;
        }
    }

    file.flush().expect("flush dense CJK benchmark file");
}
```

- [ ] **Step 2: Add benchmark**

```rust
fn bench_open_50mb_dense_cjk(c: &mut Criterion) {
    let dir = tempfile::tempdir().expect("create benchmark tempdir");
    let path = dir.path().join("bench_50mb_dense_cjk.txt");
    generate_dense_cjk_file(&path, 50);

    c.bench_function("open_50mb_dense_cjk", |b| {
        b.iter(|| {
            let (buf, meta) = core::file::load_file(&path).expect("load dense CJK benchmark file");
            assert_eq!(meta.original_encoding, None);
            std::hint::black_box((&buf, &meta));
        });
    });
}
```

Add `bench_open_50mb_dense_cjk` to the existing `criterion_group!`.

- [ ] **Step 3: Run targeted benchmark locally**

Run:

```bash
cargo bench -p edit-plus-core --bench file_loading open_50mb_dense_cjk
```

Expected:

```text
open_50mb_dense_cjk
```

The exact timing depends on machine state; the important acceptance check is that `meta.original_encoding` remains `None` and the benchmark does not enter the full-file transcode path.

## Task 6: Full Verification

**Files:**
- No new files.

**Interfaces:**
- Consumes: completed implementation.
- Produces: confidence that the regression is fixed without breaking existing encoding support.

- [ ] **Step 1: Format**

Run:

```bash
cargo fmt
```

Expected: command exits with code 0.

- [ ] **Step 2: Run core file tests**

Run:

```bash
cargo test -p edit-plus-core file::tests -- --nocapture
```

Expected:

```text
test result: ok
```

- [ ] **Step 3: Run app tests that observe `original_encoding`**

Run:

```bash
cargo test -p edit-plus-app document_view::basic_tests::from_file_records_original_encoding_for_transcoded_file -- --nocapture
```

Expected:

```text
test result: ok. 1 passed
```

- [ ] **Step 4: Compile app**

Run:

```bash
cargo check -p edit-plus-app
```

Expected:

```text
Finished `dev` profile
```

- [ ] **Step 5: Major-change verification if implementation touches wider loading behavior**

Run:

```bash
./scripts/verify.sh
```

Expected: script exits with code 0.

## Risks and Mitigations

- Risk: The simple validator allocates when previous chunk has carry bytes.
  Mitigation: The allocation happens only for split UTF-8 sequences. If benchmark regression is visible, replace it with a fixed scratch buffer in the same interface.
- Risk: `UTF-8-lossy` changes bytes if the user saves.
  Mitigation: `original_encoding` is `Some`, so the document remains dirty, matching existing transcode behavior. Follow-up work can add an explicit encoding warning in the app layer.
- Risk: Heuristic may classify rare non-UTF-8 files as UTF-8 lossy.
  Mitigation: Keep the heuristic narrow: require at least 16 valid non-ASCII UTF-8 chars and at most 4 invalid sequences.
- Risk: Early returns in `load_file()` can duplicate line-ending finalization.
  Mitigation: Extract `detect_line_ending()` before refactor and use it in every return path.

## Acceptance Checklist

- [ ] A dense CJK UTF-8 file larger than 8192 bytes loads with `original_encoding == None`.
- [ ] A dense CJK UTF-8 file larger than 2 * 64KB loads with `original_encoding == None`.
- [ ] Existing GBK and Shift_JIS tests still pass.
- [ ] Mostly UTF-8 with a sparse invalid byte remains readable and uses `UTF-8-lossy`.
- [ ] `cargo test -p edit-plus-core file::tests -- --nocapture` passes.
- [ ] `cargo check -p edit-plus-app` passes.
- [ ] If implementation scope is broad, `./scripts/verify.sh` passes.
