# 编码探测与自动转码方案

## 问题根因

`render_pipeline.rs:645` 使用 `std::str::from_utf8(&line_bytes).unwrap_or("")` 将字节转为字符串。
当文件为 GBK/GB2312/Shift_JIS 等非 UTF-8 编码时，`from_utf8` 校验失败返回空串，
导致编辑器只显示行号、内容区域空白。

加载链路现状：
```
load_file() 原样读字节 → GapBuffer 存储 → TextBuffer 包装
→ 渲染时 from_utf8 失败 → 内容显示为空
```

## 设计目标

1. 加载时自动探测编码，非 UTF-8 文件透明转码为 UTF-8
2. 转码后标记文档为 dirty（内存内容 ≠ 磁盘内容）
3. UTF-8 文件零额外开销（快速路径跳过探测）
4. 下游（TextBuffer、渲染管线、保存路径）零改动

## 依赖变更

### `crates/core/Cargo.toml`

```toml
[dependencies]
encoding_rs = "0.8"    # Mozilla 维护的编码/解码库
```

选型理由：
- `encoding_rs` 是 Firefox 的编码引擎，维护活跃，覆盖所有 CJK 编码
- 支持零拷贝 BOM 检测 + 流式解码
- 很多 Rust 生态已间接依赖（reqwest、url 等），增量编译成本低
- 不引入额外的 `chardet` 库——`encoding_rs` 自带 BOM 优先检测，
  配合 `std::str::from_utf8` 的快速路径即可覆盖绝大多数场景

## 详细修改

### 1. `crates/core/src/file.rs`

#### 1.1 `FileMetadata` 新增字段

```rust
pub struct FileMetadata {
    pub line_ending: LineEnding,
    pub had_bom: bool,
    /// 若加载时发生了编码转码，记录原始编码名称（如 "GBK", "Shift_JIS"）。
    /// UTF-8 / ASCII 文件此字段为 None。
    pub original_encoding: Option<&'static str>,
}
```

#### 1.2 新增编码探测函数

```rust
/// 检测字节序列是否为合法 UTF-8。
/// 返回 true 表示合法 UTF-8（含纯 ASCII），false 表示需要转码。
fn is_valid_utf8(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok()
}

/// 用 encoding_rs 探测编码并将字节转为 UTF-8。
///
/// 优先检查 BOM（UTF-16 LE/BE、UTF-8），无 BOM 时尝试常见中文编码。
/// 返回 (utf8_bytes, encoding_name)。
fn transcode_to_utf8(raw: &[u8]) -> (Vec<u8>, &'static str) {
    // encoding_rs::encoding_for_whatwg_label 不可靠，手动按优先级尝试
    // BOM 检测
    if raw.len() >= 2 {
        if raw[0] == 0xFF && raw[1] == 0xFE {
            let (decoded, _, _) = encoding_rs::UTF_16LE.decode(raw);
            return (decoded.into_owned().into_bytes(), "UTF-16LE");
        }
        if raw[0] == 0xFE && raw[1] == 0xFF {
            let (decoded, _, _) = encoding_rs::UTF_16BE.decode(raw);
            return (decoded.into_owned().into_bytes(), "UTF-16BE");
        }
    }

    // 无 BOM 时，按常见中文编码尝试
    // 优先级：GB18030 > GBK > Big5 > Shift_JIS > EUC-JP > EUC-KR
    // GB18030 是 GBK 超集（国标），优先尝试
    let candidates: &[(&encoding_rs::Encoding, &str)] = &[
        (encoding_rs::GB18030, "GB18030"),
        (encoding_rs::GBK, "GBK"),
        (encoding_rs::BIG5, "Big5"),
        (encoding_rs::SHIFT_JIS, "Shift_JIS"),
        (encoding_rs::EUC_JP, "EUC-JP"),
        (encoding_rs::EUC_KR, "EUC-KR"),
    ];

    for &(encoding, name) in candidates {
        let (decoded, _, had_errors) = encoding.decode(raw);
        if !had_errors {
            return (decoded.into_owned().into_bytes(), name);
        }
    }

    // 兜底：ISO-8859-1 永远不会报错
    let (decoded, _, _) = encoding_rs::ISO_8859_1.decode(raw);
    (decoded.into_owned().into_bytes(), "ISO-8859-1")
}
```

**设计考量：**
- 先做 `is_valid_utf8` 快速路径——UTF-8/ASCII 文件直接跳过，零开销
- BOM 检测优先于启发式，避免 UTF-16 文件被误判为 GBK
- 中文编码优先尝试 GB18030（国标，GBK 超集），覆盖几乎所有中文非 UTF-8 场景
- `had_errors` 作为编码匹配信号：如果某编码解码无错误，大概率就是它。
  **注意**：这不是精确匹配。GBK/GB18030 字节空间很大，Shift_JIS 文件解码为 GBK 大概率不报错，只是乱码（mojibake）。现实场景中按优先级排序的启发式已足够，精确检测需完整的统计 chardet 引擎（本次不做）。
- 兜底 ISO-8859-1 确保永远有结果

#### 1.3 修改 `load_file()` 函数

在 BOM 检测之后、写入 GapBuffer 之前，插入编码探测 + 转码逻辑。

核心策略：**首块快速判定 + 后续逐块验证**。首块非法 UTF-8 直接走转码；首块合法则继续逐块读取时验证每个 64KB chunk，任一 chunk 失败则回退到转码路径。

```rust
pub fn load_file(path: &Path) -> Result<(GapBuffer, FileMetadata), FileError> {
    // ... 现有的文件读取、null 检测、BOM 检测 ...

    let (had_bom, start) = strip_bom(&first_chunk[..n]);

    // ── 新增：编码探测（首块快速判定） ──
    let needs_transcode = !is_valid_utf8(start);

    let (transcoded_content, original_encoding) = if needs_transcode {
        // 首块不是合法 UTF-8 → 读完整文件并转码
        let mut all_bytes = Vec::with_capacity(8192 + 65536);
        all_bytes.extend_from_slice(start);
        let mut rest = Vec::new();
        file.read_to_end(&mut rest).map_err(FileError::Io)?;
        all_bytes.extend_from_slice(&rest);

        let (utf8_bytes, encoding_name) = transcode_to_utf8(&all_bytes);
        (Some(utf8_bytes), Some(encoding_name))
    } else {
        // 首块合法，但先不急着下结论——后续 chunk 也需要验证
        (None, None)
    };

    if let Some(ref content) = transcoded_content {
        // ── 转码路径：用转码后的 UTF-8 字节写入 GapBuffer ──
        scan_line_endings(content, &mut has_lf, &mut has_cr, &mut has_crlf);
        if !content.is_empty() {
            let gap = buf.allocate_gap(0, content.len(), 0);
            gap[..content.len()].copy_from_slice(content);
            buf.commit_gap(content.len());
        }
    } else {
        // ── UTF-8 快速路径：逐块读取 + 逐块验证 ──
        // 先写入首块
        if !start.is_empty() {
            scan_line_endings(start, &mut has_lf, &mut has_cr, &mut has_crlf);
            let gap = buf.allocate_gap(0, start.len(), 0);
            gap[..start.len()].copy_from_slice(start);
            buf.commit_gap(start.len());
        }

        // 逐块读取，每个 chunk 验证 UTF-8 合法性
        let chunk_size: usize = 65536;
        loop {
            let offset = buf.len();
            let gap = buf.allocate_gap(offset, chunk_size, 0);
            let to_read = gap.len().min(chunk_size);
            let n = file.read(&mut gap[..to_read]).map_err(FileError::Io)?;
            if n == 0 {
                break;
            }
            if !is_valid_utf8(&gap[..n]) {
                // 后续块验证失败 → 回退到转码路径
                // 丢弃当前 GapBuffer，重新读完整文件并转码
                drop(buf);
                drop(gap); // 释放借用
                let mut all_bytes = Vec::new();
                file.seek(std::io::SeekFrom::Start(0))?;
                file.read_to_end(&mut all_bytes).map_err(FileError::Io)?;
                let (utf8_bytes, encoding_name) = transcode_to_utf8(&all_bytes);
                transcoded_content = Some(utf8_bytes);
                original_encoding = Some(encoding_name);

                // 用转码内容重建 GapBuffer
                buf = GapBuffer::new(false).map_err(FileError::Io)?;
                let content = transcoded_content.as_ref().unwrap();
                // 重新扫描行结束符（之前的状态作废）
                has_lf = false; has_cr = false; has_crlf = false;
                scan_line_endings(content, &mut has_lf, &mut has_cr, &mut has_crlf);
                if !content.is_empty() {
                    let new_gap = buf.allocate_gap(0, content.len(), 0);
                    new_gap[..content.len()].copy_from_slice(content);
                    buf.commit_gap(content.len());
                }
                break;
            }
            scan_line_endings(&gap[..n], &mut has_lf, &mut has_cr, &mut has_crlf);
            buf.commit_gap(n);
        }
    }

    // ... 行结束符判定 ...

    Ok((buf, FileMetadata {
        line_ending,
        had_bom,
        original_encoding,
    }))
}
```

**关键细节：**
- **首块快速判定**：`is_valid_utf8(start)` 为 false 时直接走转码路径（无需逐块验证）
- **逐块验证**：首块合法不代表整个文件合法（GBK 文件可能以长段 ASCII 开头）。
  每个 64KB chunk 写入前都做 `is_valid_utf8` 校验，确保真正的 UTF-8 文件零开销，伪 UTF-8 文件被及时捕获
- **回退机制**：后续 chunk 校验失败时，`file.seek(SeekFrom::Start(0))` 回到文件头，
  重读完整文件 → 转码 → 重建 GapBuffer。此前已写入的首块数据被丢弃
- **慢路径**：首块非法 UTF-8 时，直接 `file.read_to_end()` 读剩余部分（指针已在首块之后，无需 seek）
- 转码后字节序列写入 GapBuffer，行结束符检测对转码后的字节重新执行
- 转码后 BOM 已被 `encoding_rs::decode` 消除，`had_bom` 保持原值
- **大文件注意**：非 UTF-8 文件会全量读入内存再转码，内存峰值 ≈ 原始大小 + 转码后大小。
  对几百 MB 的非 UTF-8 日志文件可能造成内存压力。后续可加阈值（如 50MB）切换为流式解码。
  TODO: 大文件流式转码优化

#### 1.4 修改所有 `FileMetadata` 构造点

`save_file()` 内部构造 `FileMetadata` 时需补上新字段（保存路径不需要编码信息，设为 `None`）：

```rust
// save_file 内部无需改动，因为它不构造 FileMetadata
// 但 DocumentView::save_as() 构造 FileMetadata 时需要补字段
```

### 2. `crates/app/src/document_view/mod.rs`

#### 2.1 `from_file()` 中根据转码标记 dirty

```rust
// 现有代码（约第 181 行）：
tb.mark_as_clean();
tb.cursor_move_to_byte(ByteIndex::ZERO);

// ... 构造 Self 的部分 ...
// 修改 dirty 字段：
dirty: _meta.original_encoding.is_some(),
```

逻辑：
- `original_encoding` 为 `Some` → 发生了转码 → `dirty: true` → 标签页显示已修改指示器
- `original_encoding` 为 `None` → 原始 UTF-8 → `dirty: false`（行为不变）
- `tb.mark_as_clean()` 仍然调用——它定义的是 TextBuffer 的增量编辑基准，与 UI 层的 dirty 无关

#### 2.2 `save_as()` 中构造 FileMetadata 补字段

```rust
// 现有代码（约第 238 行）：
let metadata = core::file::FileMetadata {
    line_ending,
    had_bom: self.had_bom,
    original_encoding: None,  // 保存时不需要编码信息
};
```

#### 2.3 新增 `original_encoding` 字段存储

在 `DocumentView` 结构体中新增字段，用于后续状态栏展示：

```rust
pub struct DocumentView {
    // ... 现有字段 ...
    /// 原始文件编码（转码加载时记录，UTF-8 文件为 None）
    pub(crate) original_encoding: Option<&'static str>,
}
```

`from_file()` 中赋值：
```rust
original_encoding: _meta.original_encoding,
```

### 3. `crates/core/src/file.rs` — 现有测试适配

所有构造 `FileMetadata` 的测试用例需补字段：

```rust
// 将所有：
FileMetadata { line_ending: LineEnding::Lf, had_bom: false }
// 改为：
FileMetadata { line_ending: LineEnding::Lf, had_bom: false, original_encoding: None }
```

### 4. 新增测试

#### 4.1 `crates/core/src/file.rs` 单元测试

```rust
#[test]
fn load_gbk_file_transcodes_to_utf8() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gbk.txt");
    // "你好" 的 GBK 编码
    std::fs::write(&path, &[
        0xC4, 0xE3, 0xBA, 0xC3, 0x0A  // "你好\n"
    ]).unwrap();

    let (buf, meta) = load_file(&path).unwrap();
    assert_eq!(meta.original_encoding, Some("GBK"));
    assert_eq!(meta.line_ending, LineEnding::Lf);
    assert!(!meta.had_bom);

    // 内容应为合法 UTF-8
    let content = read_all(&buf);
    let text = std::str::from_utf8(&content).expect("should be valid UTF-8");
    assert_eq!(text, "你好\n");
}

#[test]
fn load_utf8_file_skips_detection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("utf8.txt");
    std::fs::write(&path, "hello\n").unwrap();

    let (_, meta) = load_file(&path).unwrap();
    assert_eq!(meta.original_encoding, None);
}

#[test]
fn load_shift_jis_file_transcodes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sjis.txt");
    // "あ" 的 Shift_JIS 编码 = 0x82, 0xA0
    std::fs::write(&path, &[0x82, 0xA0, 0x0A]).unwrap();

    let (buf, meta) = load_file(&path).unwrap();
    // Shift_JIS 或 EUC-JP 都可能匹配，取决于探测顺序
    assert!(meta.original_encoding.is_some());
    let content = read_all(&buf);
    assert!(std::str::from_utf8(&content).is_ok());
}
```

#### 4.2 `crates/app/src/document_view/mod.rs` 集成测试

```rust
#[test]
fn gbk_file_marks_dirty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gbk.txt");
    std::fs::write(&path, &[0xC4, 0xE3, 0xBA, 0xC3, 0x0A]).unwrap();

    let dv = DocumentView::from_file(&path, 10, 100.0).unwrap();
    assert!(dv.dirty, "GBK transcoded file should be marked dirty");
    assert_eq!(dv.original_encoding, Some("GBK"));
}
```

## 涉及文件清单

| 文件 | 改动类型 | 说明 |
|------|----------|------|
| `crates/core/Cargo.toml` | 新增依赖 | `encoding_rs = "0.8"` |
| `crates/core/src/file.rs` | 主要改动 | `FileMetadata` 加字段、新增 `is_valid_utf8` / `transcode_to_utf8`、修改 `load_file` 流程、适配测试 |
| `crates/app/src/document_view/mod.rs` | 小改动 | `DocumentView` 加 `original_encoding` 字段、`from_file` 中 `dirty` 根据转码状态赋值、`save_as` 补字段 |

## 不改动的部分

- **渲染管线** (`render_pipeline.rs`)：GapBuffer 中始终是合法 UTF-8，`from_utf8` 不再失败
- **保存路径** (`save_file`)：原子写入逻辑不变，保存内容为 UTF-8（现代编辑器标准行为）
- **TextBuffer**：不感知编码，只处理字节 + 行结束符
- **Tab bar / 文件名**：无需改动

## 未来扩展（本次不做）

- 状态栏显示原始编码（如 "GBK → UTF-8"）
- 菜单 "Reopen with Encoding..." 手动选择编码
- 保存时可选转回原编码（round-trip 保真）
