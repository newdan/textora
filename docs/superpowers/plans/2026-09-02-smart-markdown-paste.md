# Markdown WYSIWYG Smart Paste Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 Markdown WYSIWYG 增加多表示剪贴板智能粘贴，同时保证源码视图、纯文本编辑器和“粘贴为纯文本”只消费 `text/plain`。

**Design:** `docs/specs/2026-09-02-smart-markdown-paste-design.md`

**Architecture:** `appkit-shell` 采集 `ClipboardSnapshot`，`textora-markdown::paste` 将 HTML/RTF 转成统一 `RichDocument` 并输出 Markdown，`app` 根据 `ViewPlugin::paste_preference()` 选择智能或纯文本路径，再通过现有 `EditIntent::InsertText` 和单次编辑事务插入。通用 UI 剪贴板 trait 保持纯文本，不让 UI 层依赖 app 状态或 Markdown 类型。

**Tech Stack:** Rust 2024、`clipboard-rs 0.3.5`、`scraper 0.27.0`、`ego-tree 0.11`、`url 2.5.8`、`encoding_rs 0.8`、现有 `pulldown-cmark 0.13`、Criterion、现有 `EditTransaction`。

## Global Constraints

- 产品名使用 `textora`，Markdown 包名使用 `textora-markdown`。
- `crates/ui` 不得依赖 `DocumentView`、Workspace、Commands、Events 或 app 层状态结构体。
- 源码视图、`.txt`、通用文本框和 `PastePlainText` 不得读取 HTML、RTF 或 Markdown 自定义剪贴板格式。
- `MarkdownEditorView` 是唯一返回 `PastePreference::SemanticMarkdown` 的视图。
- 只保留 `http:` / `https:` 网络图片 URL；不落盘内嵌图片，不发起网络请求。
- 普通纯文本保留单换行和空行，仅执行既有 BOM/EOL 规范化。
- HTML/RTF 转换不能证明可见文字完整时回退纯文本；显式 `text/markdown` 不做可见文字等价校验。
- 一次粘贴形成独立 undo entry；读取或转换失败不得删除当前选区。
- 不执行 HTML 脚本、事件或 CSS，不读取远端资源。
- RTF 分组深度、控制字长度使用语义常量限制，不得 panic。
- 生产函数超过 50 行时按解析、分类、输出职责拆分；不使用宽泛命名和多个 bool 表达互斥状态。
- 每个任务提交前运行该任务列出的编译或测试命令；最终运行 `./scripts/verify.sh`。

---

## File Structure

### Clipboard acquisition

- `crates/appkit-shell/src/clipboard.rs`：保留 `SystemClipboard` 纯文本适配，新增 `ClipboardSnapshot`、`DocumentClipboard` 和 `clipboard-rs` 后端。
- `crates/appkit-shell/src/lib.rs`：只重导出纯数据和剪贴板接口。

### Markdown conversion

- `crates/markdown/src/paste/mod.rs`：稳定公开入口、输入/输出类型重导出。
- `crates/markdown/src/paste/model.rs`：`RichDocument`、块/行内 enum、可见文字片段。
- `crates/markdown/src/paste/writer.rs`：统一 Markdown writer、转义和代码围栏。
- `crates/markdown/src/paste/html.rs`：HTML5 DOM 到 `RichDocument`，Office 行内 CSS 和 URL 解析。
- `crates/markdown/src/paste/rtf.rs`：受限 RTF tokenizer/parser 到 `RichDocument`。
- `crates/markdown/src/paste/selection.rs`：格式优先级、语义 HTML 判断、文字等价校验和降级原因。
- `crates/markdown/benches/paste_conversion.rs`：HTML/RTF 大文本转换基准。

### View capability and app orchestration

- `crates/ui/src/plugin.rs`：定义纯数据 `PastePreference`，默认 `PlainText`。
- `crates/markdown/src/view.rs`：WYSIWYG 返回 `SemanticMarkdown`。
- `crates/appkit-shell/src/tab_session.rs`：向 app 暴露活动插件的粘贴偏好。
- `crates/appkit-core/src/edit_command.rs`：增加 `PastePlainText`。
- `crates/appkit-shell/src/input_mapper.rs`：映射 `Cmd/Ctrl+Shift+V`。
- `crates/app/src/clipboard.rs`：把 shell snapshot 映射为 Markdown 转换输入，返回最终插入文本。
- `crates/app/src/dispatch/editor.rs`：在旧命令路径前拦截两种 Paste，进入统一事务并建立 undo merge barrier。
- `crates/app/src/commands.rs`：旧命令执行器穷尽覆盖 `PastePlainText`，只作纯文本兼容回退。
- `crates/app/src/native_menu.rs`、`crates/app/src/menu_handler.rs`：增加“粘贴并匹配样式”。

## Dependency Notes

- `clipboard-rs 0.3.5` 的官方仓库声明 Windows、macOS、Linux X11 支持纯文本、HTML、RTF 和自定义格式：https://github.com/ChurchTao/clipboard-rs
- `scraper 0.27.0` 基于 html5ever，提供 `Html::parse_fragment` 和公开 DOM tree：https://docs.rs/scraper/0.27.0/scraper/
- `ego-tree 0.11` 是 scraper 公开 DOM tree 的节点类型；本模块需要命名 `NodeRef` 才声明递归 helper：https://docs.rs/ego-tree/0.11.0/ego_tree/
- `url 2.5.8` 提供 WHATWG URL 解析及 `Url::join`：https://docs.rs/url/2.5.8/url/
- Linux 纯 Wayland 环境若 `clipboard-rs` 无可用后端，`DocumentClipboard::read_snapshot()` 返回 `None`；现有纯文本兼容性由平台验证任务实测。不得在没有真机证据时声称纯 Wayland 富格式已支持。

---

### Task 1: Declare the rich clipboard dependency

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/appkit-shell/Cargo.toml`
- Modify: `Cargo.lock`

**Interfaces:**
- Consumes: existing workspace dependency layout.
- Produces: `clipboard-rs.workspace = true` for `appkit-shell`; keeps `arboard` temporarily until Task 3.

- [ ] **Step 1: Record the current shell build baseline**

Run: `cargo check -p textora-appkit-shell`

Expected: PASS. If it fails before manifest changes, stop and report the pre-existing failure.

- [ ] **Step 2: Add the pinned workspace dependency**

Add to `[workspace.dependencies]` in `Cargo.toml`:

```toml
clipboard-rs = "0.3.5"
```

Add to `[dependencies]` in `crates/appkit-shell/Cargo.toml`:

```toml
clipboard-rs.workspace = true
```

Keep `arboard.workspace = true` during this task so production code still compiles.

- [ ] **Step 3: Resolve and audit the dependency graph**

Run: `cargo check -p textora-appkit-shell`

Expected: PASS and `Cargo.lock` contains exactly one `clipboard-rs` package entry.

Run: `cargo tree -p textora-appkit-shell -i clipboard-rs`

Expected: output shows `clipboard-rs` is consumed only by `textora-appkit-shell`.

- [ ] **Step 4: Commit the dependency approval**

```bash
git add Cargo.toml Cargo.lock crates/appkit-shell/Cargo.toml
git commit -m "build(appkit-shell): add rich clipboard dependency"
```

---

### Task 2: Add multi-representation clipboard snapshots

**Files:**
- Modify: `crates/appkit-shell/src/clipboard.rs`
- Modify: `crates/appkit-shell/src/lib.rs`

**Interfaces:**
- Consumes: `clipboard_rs::{Clipboard, ClipboardContext, ContentFormat}`.
- Produces:
  - `pub struct ClipboardSnapshot { markdown_text, html_text, rtf_bytes, plain_text, source_url }`
  - `pub trait DocumentClipboard { fn read_plain_text(&mut self) -> Option<String>; fn read_snapshot(&mut self) -> Option<ClipboardSnapshot>; }`
  - `SystemClipboard: ui::core::Clipboard + DocumentClipboard`

- [ ] **Step 1: Write failing snapshot assembly tests**

Add a private raw-source seam and tests inside `clipboard.rs`:

```rust
trait ClipboardRepresentations {
    fn available_formats(&self) -> Vec<String>;
    fn plain_text(&self) -> Option<String>;
    fn html_text(&self) -> Option<String>;
    fn rtf_bytes(&self) -> Option<Vec<u8>>;
    fn custom_bytes(&self, format: &str) -> Option<Vec<u8>>;
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
```

- [ ] **Step 2: Run the tests and verify the missing API failure**

Run: `cargo test -p textora-appkit-shell --lib clipboard::tests::snapshot_ -- --nocapture`

Expected: FAIL because `ClipboardSnapshot`, `snapshot_from`, and the fake source do not exist.

- [ ] **Step 3: Implement the pure snapshot model and alias lookup**

Use semantic constants and case-insensitive matching against the actual format names returned by the backend:

```rust
const MARKDOWN_FORMAT_ALIASES: &[&str] = &[
    "text/markdown",
    "public.markdown",
    "net.daringfireball.markdown",
];
const SOURCE_URL_FORMAT_ALIASES: &[&str] = &[
    "public.url",
    "text/x-moz-url",
    "SourceURL",
];

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
```

`snapshot_from` must read each value from the same `ClipboardRepresentations` instance, filter empty strings/byte arrays, decode custom UTF-8 with `String::from_utf8`, and take only the first line of Mozilla-style source URL data. Add pure helpers `extract_cf_html_fragment(&str) -> &str` and `extract_cf_html_source_url(&str) -> Option<String>`; marker comments take precedence over numeric offsets, and absent/malformed headers leave the original HTML untouched.

- [ ] **Step 4: Adapt `clipboard-rs` without changing UI consumers**

Implement a private `ClipboardContextRepresentations<'a>` wrapper. `SystemClipboard::read_snapshot` creates one `ClipboardContext`, wraps it, and calls `snapshot_from`. `ui::core::Clipboard::read_text` delegates to `DocumentClipboard::read_plain_text`; `write_text` calls `ClipboardContext::set_text(text.to_owned())`.

The adapter must not call `get_image`, `get_files`, or any network API.

- [ ] **Step 5: Export only the document-facing types**

Change the re-export in `crates/appkit-shell/src/lib.rs` to:

```rust
pub use clipboard::{ClipboardSnapshot, DocumentClipboard, SystemClipboard};
```

- [ ] **Step 6: Verify snapshot and existing clipboard behavior**

Run: `cargo test -p textora-appkit-shell --lib clipboard::tests -- --nocapture`

Expected: PASS.

Run: `cargo check -p textora-app`

Expected: PASS; existing TextBox/SearchBar code still sees only `ui::core::Clipboard`.

- [ ] **Step 7: Commit the shell adapter**

```bash
git add crates/appkit-shell/src/clipboard.rs crates/appkit-shell/src/lib.rs
git commit -m "feat(appkit-shell): expose rich clipboard snapshots"
```

---

### Task 3: Remove the obsolete arboard dependency

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/appkit-shell/Cargo.toml`
- Modify: `Cargo.lock`

**Interfaces:**
- Consumes: Task 2 `SystemClipboard` backed by `clipboard-rs`.
- Produces: no `arboard` dependency or source reference in the workspace clipboard path.

- [ ] **Step 1: Prove production code no longer references arboard**

Run: `rg -n "arboard::|arboard\.workspace" crates Cargo.toml`

Expected: only manifest declarations remain. If Rust source still appears, finish Task 2 before continuing.

- [ ] **Step 2: Remove both manifest declarations**

Remove `arboard` from `[workspace.dependencies]` and from `crates/appkit-shell/Cargo.toml`, then let Cargo update `Cargo.lock`.

- [ ] **Step 3: Verify the dependency is gone**

Run: `cargo check -p textora-app`

Expected: PASS.

Run: `cargo tree -p textora-app | rg "arboard"`

Expected: exit 1 with no matches.

- [ ] **Step 4: Commit dependency cleanup**

```bash
git add Cargo.toml Cargo.lock crates/appkit-shell/Cargo.toml
git commit -m "build(appkit-shell): remove plain-only clipboard backend"
```

---

### Task 4: Declare Markdown paste conversion dependencies

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/markdown/Cargo.toml`
- Modify: `Cargo.lock`

**Interfaces:**
- Produces: `scraper`, `url`, and `encoding_rs` available only to `textora-markdown`.

- [ ] **Step 1: Add exact workspace dependency floors**

Add:

```toml
scraper = "0.27.0"
ego-tree = "0.11"
url = "2.5.8"
encoding_rs = "0.8"
```

Add the corresponding `.workspace = true` entries to `crates/markdown/Cargo.toml`.

- [ ] **Step 2: Resolve and inspect ownership**

Run: `cargo check -p textora-markdown`

Expected: PASS.

Run: `cargo tree -p textora-markdown -d | rg "scraper|ego-tree|url|encoding_rs"`

Expected: command may print transitive duplicates, but the direct versions resolve to scraper 0.27.x, ego-tree 0.11.x, url 2.5.x, and encoding_rs 0.8.x. Record any duplicate major version before proceeding.

- [ ] **Step 3: Commit converter dependencies**

```bash
git add Cargo.toml Cargo.lock crates/markdown/Cargo.toml
git commit -m "build(markdown): add smart paste parser dependencies"
```

---

### Task 5: Define the rich document model

**Files:**
- Create: `crates/markdown/src/paste/mod.rs`
- Create: `crates/markdown/src/paste/model.rs`
- Modify: `crates/markdown/src/lib.rs`

**Interfaces:**
- Produces:
  - `PasteRepresentations<'a>`
  - `PreparedPaste`
  - `PasteFallbackReason`
  - `RichDocument`, `RichBlock`, `RichInline`, `ListKind`, `HeadingLevel`
  - `VisibleSegment { mode: VisibleTextMode, text: String }`

- [ ] **Step 1: Write model construction and visible-segment tests**

```rust
#[test]
fn visible_segments_keep_code_preformatted() {
    let document = RichDocument::new(vec![
        RichBlock::Paragraph(vec![RichInline::Text("before".into())]),
        RichBlock::CodeBlock { language: Some("rust".into()), text: "let  x = 1;\n".into() },
    ]);

    assert_eq!(
        document.visible_segments(),
        vec![
            VisibleSegment::flow("before"),
            VisibleSegment::preformatted("let  x = 1;\n"),
        ]
    );
}

#[test]
fn heading_level_rejects_values_outside_one_through_six() {
    assert!(HeadingLevel::try_from(0).is_err());
    assert!(HeadingLevel::try_from(7).is_err());
}
```

- [ ] **Step 2: Run tests and verify module absence**

Run: `cargo test -p textora-markdown --lib paste::model::tests -- --nocapture`

Expected: FAIL because the paste module does not exist.

- [ ] **Step 3: Implement type-driven model state**

Use enums rather than style booleans:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadingLevel { H1, H2, H3, H4, H5, H6 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Unordered,
    Ordered { start: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InlineSemantic {
    Strong,
    Emphasis,
    Strikethrough,
}

pub enum RichInline {
    Text(String),
    Strong(Vec<RichInline>),
    Emphasis(Vec<RichInline>),
    Strikethrough(Vec<RichInline>),
    InlineCode(String),
    Link { destination: String, title: Option<String>, children: Vec<RichInline> },
    RemoteImage { destination: String, title: Option<String>, alt: String },
    LineBreak,
}

pub enum RichBlock {
    Heading { level: HeadingLevel, content: Vec<RichInline> },
    Paragraph(Vec<RichInline>),
    BlockQuote(Vec<RichBlock>),
    List { kind: ListKind, items: Vec<Vec<RichBlock>> },
    CodeBlock { language: Option<String>, text: String },
    Table { header: Vec<Vec<RichInline>>, rows: Vec<Vec<Vec<RichInline>>> },
    HorizontalRule,
}
```

`visible_segments()` emits flow text in reading order and one `Preformatted` segment per code block. Links emit their label, images emit alt text, line breaks emit `\n`, and block boundaries emit flow whitespace rather than Markdown markers.

- [ ] **Step 4: Define paste input and output without shell dependencies**

```rust
pub struct PasteRepresentations<'a> {
    pub markdown: Option<&'a str>,
    pub html: Option<&'a str>,
    pub rtf: Option<&'a [u8]>,
    pub plain: Option<&'a str>,
    pub source_url: Option<&'a str>,
}

pub enum PreparedPaste {
    Markdown(String),
    HtmlConverted(String),
    RtfConverted(String),
    PlainTextFallback { text: String, reason: PasteFallbackReason },
    Empty,
}
```

Derive `Clone`, `Debug`, `PartialEq`, and `Eq` on all model/input-output enums used by tests. Keep `RichDocument.blocks` private and expose exactly these methods:

```rust
impl RichDocument {
    pub fn new(blocks: Vec<RichBlock>) -> Self;
    pub fn blocks(&self) -> &[RichBlock];
    pub fn visible_segments(&self) -> Vec<VisibleSegment>;
}

impl PreparedPaste {
    pub fn into_text(self) -> Option<String>;
}
```

Define the fallback reasons explicitly so selection and app orchestration use one vocabulary:

```rust
pub enum PasteFallbackReason {
    NoSemanticHtml,
    TextMismatch,
    HtmlParseFailed,
    RtfParseFailed,
    NoRichRepresentation,
}
```

- [ ] **Step 5: Export the paste module and verify**

Add `pub mod paste;` to `crates/markdown/src/lib.rs`.

Run: `cargo test -p textora-markdown --lib paste::model::tests`

Expected: PASS.

Run: `cargo fmt --all -- --check`

Expected: PASS.

- [ ] **Step 6: Commit the model**

```bash
git add crates/markdown/src/lib.rs crates/markdown/src/paste/mod.rs crates/markdown/src/paste/model.rs
git commit -m "feat(markdown): define rich paste document model"
```

---

### Task 6: Implement the shared Markdown writer

**Files:**
- Create: `crates/markdown/src/paste/writer.rs`
- Modify: `crates/markdown/src/paste/mod.rs`

**Interfaces:**
- Consumes: Task 5 `RichDocument`.
- Produces: `pub(crate) fn write_markdown(document: &RichDocument) -> String`.

- [ ] **Step 1: Write failing writer tests for every output family**

Include focused tests with exact output:

```rust
#[test]
fn writes_nested_inline_styles_and_escapes_plain_markers() {
    let document = RichDocument::new(vec![RichBlock::Paragraph(vec![
        RichInline::Text("literal * ".into()),
        RichInline::Strong(vec![RichInline::Emphasis(vec![RichInline::Text("both".into())])]),
    ])]);
    assert_eq!(write_markdown(&document), r"literal \* ***both***");
}

#[test]
fn code_fence_is_longer_than_backticks_in_content() {
    let document = RichDocument::new(vec![RichBlock::CodeBlock {
        language: Some("rust".into()),
        text: "let marker = ```;\n".into(),
    }]);
    assert_eq!(write_markdown(&document), "````rust\nlet marker = ```;\n````");
}

#[test]
fn writes_gfm_table_and_nested_list() {
    let text = |value: &str| vec![RichInline::Text(value.into())];
    let document = RichDocument::new(vec![
        RichBlock::Table {
            header: vec![text("Name"), text("Value")],
            rows: vec![vec![text("A"), text("1")]],
        },
        RichBlock::List {
            kind: ListKind::Unordered,
            items: vec![vec![
                RichBlock::Paragraph(text("parent")),
                RichBlock::List {
                    kind: ListKind::Ordered { start: 1 },
                    items: vec![vec![RichBlock::Paragraph(text("child"))]],
                },
            ]],
        },
    ]);

    assert_eq!(
        write_markdown(&document),
        "| Name | Value |\n| --- | --- |\n| A | 1 |\n\n- parent\n  1. child"
    );
}
```

- [ ] **Step 2: Verify tests fail**

Run: `cargo test -p textora-markdown --lib paste::writer::tests -- --nocapture`

Expected: FAIL because `write_markdown` does not exist.

- [ ] **Step 3: Implement block writing with one responsibility per helper**

Required helpers:

```rust
fn write_blocks(blocks: &[RichBlock], output: &mut String, nesting: NestingContext);
fn write_block(block: &RichBlock, output: &mut String, nesting: NestingContext);
fn write_list(kind: ListKind, items: &[Vec<RichBlock>], output: &mut String, nesting: NestingContext);
fn write_table(header: &[Vec<RichInline>], rows: &[Vec<Vec<RichInline>>], output: &mut String);
fn write_inlines(inlines: &[RichInline], output: &mut String, context: InlineContext);
fn longest_backtick_run(text: &str) -> usize;
```

`NestingContext` and `InlineContext` must be enums/structs with named fields, not positional bool arguments. Join top-level blocks with exactly `\n\n`; do not add a final newline.

- [ ] **Step 4: Implement context-aware escaping and safe URLs**

Escape Markdown control characters only in `RichInline::Text`; never alter `InlineCode` or `CodeBlock` contents. Table cells additionally escape `|` and replace line breaks with `<br>`. Choose a backtick delimiter whose length is `longest_backtick_run + 1`, with a minimum of one for inline code and three for fenced blocks.

- [ ] **Step 5: Run writer and parser round-trip tests**

Add this concrete fixture and round-trip test:

```rust
fn representative_document() -> RichDocument {
    let text = |value: &str| vec![RichInline::Text(value.into())];
    RichDocument::new(vec![
        RichBlock::Heading { level: HeadingLevel::H2, content: text("Heading") },
        RichBlock::List {
            kind: ListKind::Unordered,
            items: vec![vec![
                RichBlock::Paragraph(text("parent")),
                RichBlock::List {
                    kind: ListKind::Ordered { start: 1 },
                    items: vec![vec![RichBlock::Paragraph(text("child"))]],
                },
            ]],
        },
        RichBlock::BlockQuote(vec![RichBlock::Paragraph(text("quoted"))]),
        RichBlock::Table {
            header: vec![text("Name")],
            rows: vec![vec![text("A")]],
        },
        RichBlock::CodeBlock { language: Some("rust".into()), text: "let x = 1;".into() },
        RichBlock::Paragraph(vec![RichInline::Link {
            destination: "https://example.com".into(),
            title: None,
            children: text("link"),
        }]),
    ])
}

#[test]
fn representative_writer_output_reparses_without_visible_text_loss() {
    let document = representative_document();
    let markdown = write_markdown(&document);
    let parsed = crate::parser::parse_markdown(&markdown);

    assert!(!parsed.events.is_empty());
    for visible in ["Heading", "parent", "child", "quoted", "Name", "let x = 1", "link"] {
        assert!(markdown.contains(visible), "writer output lost {visible:?}: {markdown:?}");
    }
}
```

Run: `cargo test -p textora-markdown --lib paste::writer::tests`

Expected: PASS.

- [ ] **Step 6: Commit the writer**

```bash
git add crates/markdown/src/paste/mod.rs crates/markdown/src/paste/writer.rs
git commit -m "feat(markdown): write rich paste trees as markdown"
```

---

### Task 7: Parse semantic HTML and Office inline styles

**Files:**
- Create: `crates/markdown/src/paste/html.rs`
- Modify: `crates/markdown/src/paste/mod.rs`

**Interfaces:**
- Produces:
  - `pub(crate) struct HtmlConversion { pub document: RichDocument, pub semantic_markup: SemanticMarkup }`
  - `pub(crate) fn parse_html(html: &str, source_url: Option<&str>) -> Result<HtmlConversion, HtmlPasteError>`
  - `HtmlPasteError::NestingDepthExceeded` for bounded conversion traversal.

- [ ] **Step 1: Write failing semantic HTML tests**

```rust
#[test]
fn parses_browser_blocks_inline_styles_links_and_remote_images() {
    let conversion = parse_html(
        r#"<h2>Title</h2><p><strong>bold</strong> <a href="../a">link</a>
            <img src="img.png" alt="diagram"></p><ul><li>one</li><li>two</li></ul>"#,
        Some("https://example.com/docs/page"),
    ).expect("valid HTML fixture");

    assert_eq!(conversion.semantic_markup, SemanticMarkup::Present);
    assert_eq!(
        write_markdown(&conversion.document),
        "## Title\n\n**bold** [link](https://example.com/a) ![diagram](https://example.com/docs/img.png)\n\n- one\n- two"
    );
}

#[test]
fn office_inline_css_maps_only_supported_semantics() {
    let conversion = parse_html(
        r#"<p><span style="font-weight:700;color:red">bold</span>
            <span style="font-style:italic;text-decoration:line-through">both</span></p>"#,
        None,
    ).expect("valid Office HTML fixture");
    assert_eq!(write_markdown(&conversion.document), "**bold** *~~both~~*");
}

#[test]
fn highlight_only_spans_are_not_semantic_markup() {
    let conversion = parse_html(
        r#"<div><span style="color:#f00"># source</span></div>"#,
        None,
    ).expect("valid highlighted source HTML");
    assert_eq!(conversion.semantic_markup, SemanticMarkup::Absent);
}
```

- [ ] **Step 2: Run tests and verify failure**

Run: `cargo test -p textora-markdown --lib paste::html::tests -- --nocapture`

Expected: FAIL because the HTML conversion API is missing.

- [ ] **Step 3: Implement HTML5 fragment traversal**

Use `scraper::Html::parse_fragment`, `scraper::Node`, `ElementRef::wrap`, and small helpers:

```rust
pub(crate) const MAX_HTML_NESTING_DEPTH: usize = 256;

fn ensure_dom_depth_within_limit(root: NodeRef<'_, Node>) -> Result<(), HtmlPasteError>;
fn parse_block_node(node: NodeRef<'_, Node>, base_url: Option<&Url>) -> Vec<RichBlock>;
fn parse_inline_children(node: NodeRef<'_, Node>, base_url: Option<&Url>) -> Vec<RichInline>;
fn parse_list(element: ElementRef<'_>, kind: ListKind, base_url: Option<&Url>) -> RichBlock;
fn parse_table(element: ElementRef<'_>, base_url: Option<&Url>) -> Option<RichBlock>;
fn inline_semantics(element: ElementRef<'_>) -> Vec<InlineSemantic>;
fn resolve_destination(raw: &str, base_url: Option<&Url>) -> Option<Url>;
```

`ensure_dom_depth_within_limit` walks with an explicit `(node, depth)` stack before recursive conversion and returns `NestingDepthExceeded` above the constant. Skip `script`, `style`, `template`, `[hidden]`, `aria-hidden="true"`, `display:none`, and `visibility:hidden`. Unknown elements preserve visible children. Do not evaluate stylesheets; only parse the element's own `style` declarations for `font-weight`, `font-style`, `text-decoration`, `display`, and `visibility`. Apply supported inline wrappers in the deterministic order Strong → Emphasis → Strikethrough.

- [ ] **Step 4: Implement image and URL policy**

Links accept `http`, `https`, and `mailto`; unsupported or invalid schemes emit label text without a link. Images accept only `http` and `https`; rejected images emit alt text as `RichInline::Text` when non-empty. Relative destinations require a valid absolute `source_url` and use `Url::join`.

- [ ] **Step 5: Cover malformed HTML, tables, quotes, code and nested lists**

Add these table-driven and exact-output tests:

```rust
#[test]
fn malformed_html_remains_parseable_and_preserves_text() {
    let conversion = parse_html("<p>one<strong>two<p>three", None)
        .expect("html5ever recovers malformed fragments");
    assert_eq!(write_markdown(&conversion.document), "one**two**\n\nthree");
}

#[test]
fn converts_quote_list_code_and_table() {
    let html = r#"<blockquote><ul><li>quoted</li></ul></blockquote>
        <pre><code class="language-rust">let x = 1;</code></pre>
        <table><thead><tr><th>Name</th></tr></thead>
        <tbody><tr><td>A</td></tr></tbody></table>"#;
    let conversion = parse_html(html, None).expect("valid structural fixture");
    assert_eq!(
        write_markdown(&conversion.document),
        "> - quoted\n\n```rust\nlet x = 1;\n```\n\n| Name |\n| --- |\n| A |"
    );
}

#[test]
fn rejects_embedded_images_and_active_content() {
    for source in ["data:image/png;base64,AA", "file:///tmp/a.png", "cid:image001"] {
        let html = format!(r#"<script>bad()</script><img src="{source}" alt="diagram" onload="bad()">"#);
        let conversion = parse_html(&html, None).expect("invalid image schemes degrade to alt text");
        assert_eq!(write_markdown(&conversion.document), "diagram");
    }
}

#[test]
fn excessive_html_depth_returns_a_typed_error() {
    let html = format!(
        "{}text{}",
        "<div>".repeat(MAX_HTML_NESTING_DEPTH + 1),
        "</div>".repeat(MAX_HTML_NESTING_DEPTH + 1),
    );
    assert_eq!(parse_html(&html, None), Err(HtmlPasteError::NestingDepthExceeded));
}
```

Run: `cargo test -p textora-markdown --lib paste::html::tests`

Expected: PASS with no panic.

- [ ] **Step 6: Commit HTML conversion**

```bash
git add crates/markdown/src/paste/mod.rs crates/markdown/src/paste/html.rs
git commit -m "feat(markdown): convert semantic html for paste"
```

---

### Task 8: Parse the supported RTF subset safely

**Files:**
- Create: `crates/markdown/src/paste/rtf.rs`
- Modify: `crates/markdown/src/paste/mod.rs`

**Interfaces:**
- Produces: `pub(crate) fn parse_rtf(input: &[u8]) -> Result<RichDocument, RtfPasteError>`.

- [ ] **Step 1: Write failing RTF behavior and limit tests**

```rust
#[test]
fn parses_paragraphs_unicode_inline_styles_and_hyperlinks() {
    let input = br#"{\rtf1\ansi\ansicpg1252
        First \b bold\b0 \i italic\i0\par
        Unicode \u20320?\u22909? \field{\*\fldinst HYPERLINK "https://example.com"}{\fldrslt link}}
    }"#;
    let document = parse_rtf(input).expect("supported RTF fixture");
    assert_eq!(
        write_markdown(&document),
        "First **bold** *italic*\n\nUnicode 你好 [link](https://example.com)"
    );
}

#[test]
fn excessive_group_depth_returns_a_typed_error() {
    let input = "{".repeat(MAX_RTF_GROUP_DEPTH + 1);
    assert_eq!(parse_rtf(input.as_bytes()), Err(RtfPasteError::GroupDepthExceeded));
}

#[test]
fn cells_and_rows_degrade_to_ordered_visible_text() {
    let document = parse_rtf(br"{\rtf1 a\cell b\cell\row}")
        .expect("table controls degrade safely");
    assert_eq!(write_markdown(&document), "a\tb");
}
```

- [ ] **Step 2: Verify the tests fail**

Run: `cargo test -p textora-markdown --lib paste::rtf::tests -- --nocapture`

Expected: FAIL because `parse_rtf` is missing.

- [ ] **Step 3: Implement a bounded tokenizer**

Use constants and explicit token state:

```rust
const MAX_RTF_GROUP_DEPTH: usize = 256;
const MAX_RTF_CONTROL_WORD_BYTES: usize = 64;

enum RtfToken<'a> {
    GroupStart,
    GroupEnd,
    Control { name: &'a str, argument: Option<i32> },
    EscapedByte(u8),
    Text(&'a [u8]),
}
```

The tokenizer handles `{`, `}`, `\\`, `\{`, `\}`, `\'hh`, control words with optional signed numeric arguments, and one delimiter space. It returns typed errors for invalid hex, unmatched groups, excessive depth and oversized control words.

- [ ] **Step 4: Implement parser state and Unicode/code-page decoding**

```rust
struct RtfState {
    destination: RtfDestination,
    inline_styles: Vec<InlineSemantic>,
    unicode_fallback_bytes: usize,
    ansi_code_page: &'static encoding_rs::Encoding,
}
```

Support `\b`, `\i`, `\strike`, `\par`, `\line`, `\tab`, `\cell`, `\row`, `\ucN`, `\uN`, `\ansicpgN`, `\fldinst`, `\fldrslt`, `\listtext`, and `\pntext`. Combine consecutive signed `\uN` UTF-16 surrogate units before decoding and skip exactly the configured `\ucN` fallback bytes. Unknown starred destinations are skipped. Unsupported code pages use Windows-1252 only when RTF declares generic `\ansi`; otherwise return `UnsupportedCodePage` so selection can fall back to plain text.

- [ ] **Step 5: Add malformed and fallback coverage**

Add concrete tests with these assertions:

```rust
#[test]
fn malformed_tokens_return_typed_errors() {
    assert_eq!(parse_rtf(br"{\rtf1 text"), Err(RtfPasteError::UnclosedGroup));
    assert_eq!(parse_rtf(br"{\rtf1 \'zz}"), Err(RtfPasteError::InvalidHexEscape));
}

#[test]
fn unicode_fallback_and_strike_are_decoded_once() {
    let document = parse_rtf(br"{\rtf1\uc1 \u-10180?\u-8435? \strike gone\strike0}")
        .expect("signed UTF-16 units and strike are supported");
    assert_eq!(write_markdown(&document), "🌍 ~~gone~~");
}

#[test]
fn list_destinations_and_pictures_degrade_safely() {
    let document = parse_rtf(br"{\rtf1{\pntext\'b7\tab}item\par{\pict ignored}after}")
        .expect("simple bullet and skipped picture are supported");
    assert_eq!(write_markdown(&document), "- item\n\nafter");
}
```

Run: `cargo test -p textora-markdown --lib paste::rtf::tests`

Expected: PASS with no panic.

- [ ] **Step 6: Commit RTF conversion**

```bash
git add crates/markdown/src/paste/mod.rs crates/markdown/src/paste/rtf.rs
git commit -m "feat(markdown): parse safe rtf paste subset"
```

---

### Task 9: Select the best representation and enforce text equivalence

**Files:**
- Create: `crates/markdown/src/paste/selection.rs`
- Modify: `crates/markdown/src/paste/mod.rs`

**Interfaces:**
- Consumes: Tasks 5–8 model, HTML parser, RTF parser and writer.
- Produces: `pub fn prepare_paste(input: PasteRepresentations<'_>) -> PreparedPaste`.

- [ ] **Step 1: Write a table-driven failing priority suite**

Cover these exact cases:

```rust
#[test]
fn explicit_markdown_wins_without_visible_text_comparison() {
    let prepared = prepare_paste(PasteRepresentations {
        markdown: Some("**source**"),
        html: Some("<strong>source</strong>"),
        rtf: None,
        plain: Some("source"),
        source_url: None,
    });
    assert_eq!(prepared, PreparedPaste::Markdown("**source**".into()));
}

#[test]
fn semantic_html_wins_when_visible_text_matches() {
    let prepared = prepare_paste(PasteRepresentations {
        markdown: None,
        html: Some("<p><strong>same</strong></p>"),
        rtf: None,
        plain: Some("same"),
        source_url: None,
    });
    assert_eq!(prepared, PreparedPaste::HtmlConverted("**same**".into()));
}

#[test]
fn highlighted_markdown_source_uses_plain_text() {
    let prepared = prepare_paste(PasteRepresentations {
        markdown: None,
        html: Some("<div><span style='color:red'># source</span></div>"),
        rtf: Some(br"{\rtf1\cf1 # source}"),
        plain: Some("# source"),
        source_url: None,
    });
    assert_eq!(prepared, PreparedPaste::PlainTextFallback {
        text: "# source".into(),
        reason: PasteFallbackReason::NoSemanticHtml,
    });
}
```

Add this named test after the three primary cases:

```rust
#[test]
fn mismatch_rtf_and_empty_cases_follow_the_priority_contract() {
    assert!(matches!(
        prepare_paste(PasteRepresentations {
            markdown: None,
            html: Some("<p>different</p>"),
            rtf: None,
            plain: Some("plain"),
            source_url: None,
        }),
        PreparedPaste::PlainTextFallback { reason: PasteFallbackReason::TextMismatch, .. }
    ));

    assert!(matches!(
        prepare_paste(PasteRepresentations {
            markdown: None,
            html: None,
            rtf: Some(br"{\rtf1\b rich\b0}"),
            plain: Some("rich"),
            source_url: None,
        }),
        PreparedPaste::RtfConverted(ref text) if text == "**rich**"
    ));

    assert_eq!(
        prepare_paste(PasteRepresentations {
            markdown: None,
            html: None,
            rtf: None,
            plain: None,
            source_url: None,
        }),
        PreparedPaste::Empty
    );
}

#[test]
fn unsafe_html_depth_falls_through_to_rtf() {
    let html = format!(
        "{}rich{}",
        "<div>".repeat(crate::paste::html::MAX_HTML_NESTING_DEPTH + 1),
        "</div>".repeat(crate::paste::html::MAX_HTML_NESTING_DEPTH + 1),
    );
    let prepared = prepare_paste(PasteRepresentations {
        markdown: None,
        html: Some(&html),
        rtf: Some(br"{\rtf1\b rich\b0}"),
        plain: Some("rich"),
        source_url: None,
    });
    assert_eq!(prepared, PreparedPaste::RtfConverted("**rich**".into()));
}

#[test]
fn rich_representation_is_used_when_plain_is_absent() {
    let prepared = prepare_paste(PasteRepresentations {
        markdown: None,
        html: Some("<p><strong>rich</strong></p>"),
        rtf: None,
        plain: None,
        source_url: None,
    });
    assert_eq!(prepared, PreparedPaste::HtmlConverted("**rich**".into()));
}
```

- [ ] **Step 2: Run and verify missing orchestrator failure**

Run: `cargo test -p textora-markdown --lib paste::selection::tests -- --nocapture`

Expected: FAIL because `prepare_paste` is missing.

- [ ] **Step 3: Implement deterministic selection**

Use this exact decision order:

```text
non-empty explicit Markdown -> Markdown
semantic HTML parse success + plain absent or equivalent visible text -> HtmlConverted
semantic HTML parse success + mismatch -> PlainTextFallback(TextMismatch)
non-semantic HTML + plain exists -> PlainTextFallback(NoSemanticHtml)
non-semantic HTML + plain absent -> try RTF
HTML parse failure or absent -> try RTF
RTF success + plain absent or equivalent visible text -> RtfConverted
RTF mismatch/failure + plain exists -> PlainTextFallback(reason)
no rich success + non-empty plain -> PlainTextFallback(NoRichRepresentation)
otherwise -> Empty
```

On HTML failure, remember `HtmlParseFailed`; if no RTF succeeds, use it as the plain fallback reason. On RTF failure use `RtfParseFailed`. Any converted Markdown that is empty after writing becomes `Empty`, not an insertion. Do not use source application names or numerical style scores.

- [ ] **Step 4: Implement visible-text equivalence**

Required helpers:

```rust
fn equivalent_visible_text(document: &RichDocument, plain: &str) -> bool;
fn normalize_flow_text(text: &str) -> String;
fn normalize_line_endings(text: &str) -> String;
fn preformatted_segments_appear_in_order(document: &RichDocument, plain: &str) -> bool;
```

`normalize_flow_text` strips a leading BOM, normalizes CRLF/CR, maps NBSP to space, and collapses Unicode whitespace. In addition to normalized whole-text equality, every `Preformatted` segment must appear byte-for-byte after EOL normalization in the plain text and in document order.

- [ ] **Step 5: Verify priority, Unicode and line break behavior**

Add the following exact tests:

```rust
#[test]
fn plain_text_keeps_line_breaks_and_unicode_bytes() {
    let plain = "a\nb\n\nc 你好 🌍 e\u{301}";
    let prepared = prepare_paste(PasteRepresentations {
        markdown: None,
        html: None,
        rtf: None,
        plain: Some(plain),
        source_url: None,
    });
    assert!(matches!(
        prepared,
        PreparedPaste::PlainTextFallback { ref text, .. } if text.as_bytes() == plain.as_bytes()
    ));
}

#[test]
fn preformatted_whitespace_must_match_exactly() {
    let document = RichDocument::new(vec![RichBlock::CodeBlock {
        language: None,
        text: "let  x".into(),
    }]);
    assert!(!equivalent_visible_text(&document, "let x"));
}

#[test]
fn non_breaking_space_is_equivalent_in_flow_text() {
    let document = RichDocument::new(vec![RichBlock::Paragraph(vec![RichInline::Text(
        "a\u{a0}b".into(),
    )])]);
    assert!(equivalent_visible_text(&document, "a b"));
}
```

URL resolution is covered by the Task 7 test that resolves `../a` from a fixed base URL; the converter has no filesystem or HTTP dependency, so no I/O mock is introduced.

Run: `cargo test -p textora-markdown --lib paste::selection::tests`

Expected: PASS.

Run: `cargo test -p textora-markdown --lib paste::`

Expected: all paste tests PASS.

- [ ] **Step 6: Commit representation selection**

```bash
git add crates/markdown/src/paste/mod.rs crates/markdown/src/paste/selection.rs
git commit -m "feat(markdown): select safest clipboard representation"
```

---

### Task 10: Advertise semantic paste capability from WYSIWYG only

**Files:**
- Modify: `crates/ui/src/plugin.rs`
- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/appkit-shell/src/tab_session.rs`

**Interfaces:**
- Produces: `PastePreference::{PlainText, SemanticMarkdown}` and `TabSession::paste_preference()`.

- [ ] **Step 1: Write failing default and WYSIWYG capability tests**

In `ui::plugin` tests, assert a minimal plugin returns `PlainText`. In Markdown view tests:

```rust
#[test]
fn markdown_editor_is_the_only_markdown_view_requesting_semantic_paste() {
    assert_eq!(MarkdownEditorView::new().paste_preference(), PastePreference::SemanticMarkdown);
    assert_eq!(MarkdownView::new().paste_preference(), PastePreference::PlainText);
}
```

- [ ] **Step 2: Run and verify enum/method absence**

Run: `cargo test -p textora-markdown --lib markdown_editor_is_the_only_markdown_view_requesting_semantic_paste`

Expected: FAIL because `PastePreference` and `paste_preference` do not exist.

- [ ] **Step 3: Add the pure-data capability**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PastePreference {
    PlainText,
    SemanticMarkdown,
}
```

Add to `ViewPlugin`:

```rust
fn paste_preference(&self) -> PastePreference {
    PastePreference::PlainText
}
```

Override only in `impl ViewPlugin for MarkdownEditorView`.

- [ ] **Step 4: Add a session forwarding method**

```rust
pub fn paste_preference(&self) -> ui::plugin::PastePreference {
    self.runtime.plugin.paste_preference()
}
```

- [ ] **Step 5: Verify architecture and behavior**

Run: `cargo test -p textora-markdown --lib markdown_editor_is_the_only_markdown_view_requesting_semantic_paste`

Expected: PASS.

Run: `cargo test -p textora-ui --lib plugin`

Expected: PASS.

Run: `./scripts/check_architecture.sh`

Expected: PASS; UI still has no app dependency.

- [ ] **Step 6: Commit the capability**

```bash
git add crates/ui/src/plugin.rs crates/markdown/src/view.rs crates/appkit-shell/src/tab_session.rs
git commit -m "feat(ui): declare view paste preference"
```

---

### Task 11: Add the plain-text paste command and shortcut

**Files:**
- Modify: `crates/appkit-core/src/edit_command.rs`
- Modify: `crates/appkit-shell/src/input_mapper.rs`

**Interfaces:**
- Produces: `EditCommand::PastePlainText`; maps modified V before ordinary Paste.

- [ ] **Step 1: Write the failing shortcut test**

```rust
#[test]
fn cmd_shift_v_pastes_plain_text() {
    let key = Key::Character("v".into());
    assert_eq!(key_to_command(&key, cmd_shift()), Some(EditCommand::PastePlainText));
}
```

Keep the existing `cmd_v_paste` assertion unchanged.

- [ ] **Step 2: Run and verify the missing variant failure**

Run: `cargo test -p textora-appkit-shell --lib input_mapper::tests::cmd_shift_v_pastes_plain_text`

Expected: FAIL because `PastePlainText` is undefined.

- [ ] **Step 3: Add the command and ordered mapping**

Add `PastePlainText` next to `Paste`. In the super-key match, place:

```rust
"v" if shift => Some(EditCommand::PastePlainText),
"v" => Some(EditCommand::Paste),
```

before any generic V branch.

- [ ] **Step 4: Verify both shortcuts and exhaustive matches**

Run: `cargo test -p textora-appkit-shell --lib input_mapper::tests::cmd_`

Expected: ordinary V and Shift+V tests PASS.

Run: `cargo check -p textora-app`

Expected: initially identifies every exhaustive match that needs the new variant. Add explicit `PastePlainText` arms only where the compiler requires; do not use wildcard arms to hide command semantics.

- [ ] **Step 5: Commit command mapping**

```bash
git add crates/appkit-core/src/edit_command.rs crates/appkit-shell/src/input_mapper.rs
git commit -m "feat(input): add paste as plain text command"
```

---

### Task 12: Add native menu access to plain-text paste

**Files:**
- Modify: `crates/app/src/native_menu.rs`
- Modify: `crates/app/src/menu_handler.rs`

**Interfaces:**
- Consumes: Task 11 `EditCommand::PastePlainText`.
- Produces: `MenuAction::PastePlainText`, macOS tag 29, title “粘贴并匹配样式”, key equivalent `V`.

- [ ] **Step 1: Write failing menu dispatch test**

```rust
#[test]
fn paste_plain_text_menu_maps_to_edit_command() {
    let commands = dispatch_menu_action(MenuAction::PastePlainText);
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        &commands[0],
        AppCommand::Edit(EditCommand::PastePlainText)
    ));
}
```

- [ ] **Step 2: Run and verify missing menu action failure**

Run: `cargo test -p textora-app --lib menu_handler::tests::paste_plain_text_menu_maps_to_edit_command`

Expected: FAIL because `MenuAction::PastePlainText` is missing.

- [ ] **Step 3: Add the menu action, tag and item**

Add tag mapping `29 => MenuAction::PastePlainText` and place this item immediately after ordinary paste:

```rust
m.addItem(&make_item("粘贴并匹配样式", 29, "V", target, mtm));
```

Map the action to `EditCommand::PastePlainText` in `dispatch_menu_action`.

- [ ] **Step 4: Verify menu tests and macOS compilation**

Run: `cargo test -p textora-app --lib menu_handler::tests`

Expected: PASS.

Run: `cargo check -p textora-app`

Expected: PASS on the current macOS host.

- [ ] **Step 5: Commit menu integration**

```bash
git add crates/app/src/native_menu.rs crates/app/src/menu_handler.rs
git commit -m "feat(app): expose paste as plain text menu action"
```

---

### Task 13: Route both paste commands through one transaction

**Files:**
- Modify: `crates/app/src/clipboard.rs`
- Modify: `crates/app/src/dispatch/editor.rs`
- Modify: `crates/app/src/commands.rs`

**Interfaces:**
- Consumes: `DocumentClipboard`, `ClipboardSnapshot`, `PastePreference`, `prepare_paste`.
- Produces:
  - `PasteRequestKind::{Smart, PlainText}`
  - `prepare_document_paste(...) -> Option<String>`
  - `App::dispatch_document_paste(...) -> AppEffect`
  - test-only `App::dispatch_document_paste_with_clipboard_for_test(...)`

- [ ] **Step 1: Write failing policy tests with an injected clipboard**

Inside `crates/app/src/clipboard.rs`, add this crate-visible test double above the test module so sibling dispatch tests can reuse it:

```rust
#[cfg(test)]
pub(crate) struct TestDocumentClipboard {
    snapshot: Option<ClipboardSnapshot>,
    plain_text: Option<String>,
    pub(crate) plain_reads: usize,
    pub(crate) snapshot_reads: usize,
}

#[cfg(test)]
impl TestDocumentClipboard {
    pub(crate) fn empty() -> Self;
    pub(crate) fn with_plain(plain: &str) -> Self;
    pub(crate) fn with_html(html: &str, plain: &str) -> Self;
    pub(crate) fn with_all_formats() -> Self;
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
```

`with_plain` fills both `plain_text` and the snapshot's `plain_text`; `with_html` adds HTML to that snapshot; `with_all_formats` uses plain text `"plain\ntext"` and non-empty Markdown, HTML and RTF fixtures. Add exact policy tests:

```rust
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

#[test]
fn wysiwyg_smart_paste_converts_html() {
    let mut clipboard = TestDocumentClipboard::with_html("<p><strong>rich</strong></p>", "rich");
    let text = prepare_document_paste(
        &mut clipboard,
        PastePreference::SemanticMarkdown,
        PasteRequestKind::Smart,
    );
    assert_eq!(text.as_deref(), Some("**rich**"));
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
```

Update the existing ownership test in the same file so it asserts the shell source contains `clipboard_rs::ClipboardContext` and contains no `arboard::`; keep its assertion that app sources do not own a platform clipboard backend.

- [ ] **Step 2: Run and verify missing policy API failure**

Run: `cargo test -p textora-app --lib clipboard::tests -- --nocapture`

Expected: FAIL because `PasteRequestKind` and `prepare_document_paste` do not exist.

- [ ] **Step 3: Implement mode-aware preparation**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PasteRequestKind {
    Smart,
    PlainText,
}
```

For plain paths, call only `DocumentClipboard::read_plain_text`. For semantic smart paste, call `read_snapshot` once, map borrowed fields into `textora_markdown::paste::PasteRepresentations`, call `prepare_paste`, extract its string, then call `normalize_paste_text`. Under `#[cfg(not(feature = "markdown"))]`, semantic preparation returns the snapshot's plain text; do not add a hard dependency.

- [ ] **Step 4: Add transactional dispatch with merge barriers**

In `dispatch/editor.rs`, intercept both paste commands immediately before `edit_intent_for_command`:

```rust
if matches!(cmd, EditCommand::Paste | EditCommand::PastePlainText) {
    return self.dispatch_document_paste(&cmd, Some(event_loop));
}
```

`dispatch_document_paste` must:

1. read the active tab's `paste_preference`;
2. prepare text before mutating the document;
3. return `AppEffect::NONE` when preparation is empty;
4. call `document.break_edit_merge()` before dispatch;
5. call `dispatch_transactional_edit(EditIntent::InsertText(text), event_loop)`;
6. call `document.break_edit_merge()` after dispatch so following typing cannot merge with the paste.

Add a test-only dispatch seam, import `crate::clipboard::TestDocumentClipboard` once in the `edit_tests` module, and add a concrete undo-isolation test:

```rust
#[cfg(test)]
pub(crate) fn dispatch_document_paste_with_clipboard_for_test(
    &mut self,
    command: &EditCommand,
    clipboard: &mut dyn DocumentClipboard,
) -> AppEffect {
    self.dispatch_document_paste_with_clipboard(command, clipboard, None)
}

#[test]
fn paste_is_an_independent_undo_entry_between_typing_runs() {
    let mut app = App::new(None);
    app.push_entry_for_test(
        DocumentView::new(vec![String::new()], 40, 40.0),
        Box::new(crate::plugins::editor::EditorPlugin::new()),
    );
    app.switch_workspace_for_test(0);

    app.dispatch_transactional_edit_for_test(EditCommand::InsertText("a".into()));
    let mut clipboard = TestDocumentClipboard::with_plain("b");
    app.dispatch_document_paste_with_clipboard_for_test(&EditCommand::Paste, &mut clipboard);
    app.dispatch_transactional_edit_for_test(EditCommand::InsertText("c".into()));

    assert_eq!(active_text(&app), "abc");
    dispatch_undo(&mut app);
    assert_eq!(active_text(&app), "ab");
    dispatch_undo(&mut app);
    assert_eq!(active_text(&app), "a");
    dispatch_undo(&mut app);
    assert_eq!(active_text(&app), "");
}
```

Define the helpers explicitly:

```rust
fn active_text(app: &App) -> String {
    app.active_tab_session().expect("active tab should exist").document.full_text()
}

fn dispatch_undo(app: &mut App) {
    app.active_tab_session_mut().expect("active tab should exist").document.undo();
}

fn dispatch_redo(app: &mut App) {
    app.active_tab_session_mut().expect("active tab should exist").document.redo();
}
```

These call the document's public undo/redo behavior and never inspect or mutate its internal history stack.

- [ ] **Step 5: Preserve the legacy command executor as plain-only fallback**

In `commands.rs`, match both `Paste` and `PastePlainText` to the existing plain-text `paste_from_clipboard`. This path exists for isolated legacy tests; production editor dispatch must be covered by tests proving it intercepts first.

- [ ] **Step 6: Test selection safety and atomic undo**

Add these named tests with exact before/after assertions:

```rust
#[test]
fn paste_replaces_forward_and_backward_selection_once() {
    for (anchor, cursor) in [(1, 4), (4, 1)] {
        let mut app = app_with_text("hello");
        set_active_selection(&mut app, anchor, cursor);
        let mut clipboard = TestDocumentClipboard::with_plain("X");
        app.dispatch_document_paste_with_clipboard_for_test(&EditCommand::Paste, &mut clipboard);
        assert_eq!(active_text(&app), "hXo");
        assert_eq!(active_selection(&app), None);
        assert_eq!(active_cursor(&app), 2);
    }
}

#[test]
fn failed_clipboard_read_preserves_selection_and_cursor() {
    let mut app = app_with_text("hello");
    set_active_selection(&mut app, 1, 4);
    let before = active_document_snapshot(&app);
    let mut clipboard = TestDocumentClipboard::empty();
    app.dispatch_document_paste_with_clipboard_for_test(&EditCommand::Paste, &mut clipboard);
    assert_eq!(active_document_snapshot(&app), before);
}

#[test]
fn text_mismatch_falls_back_and_undo_redo_remains_atomic() {
    let mut app = app_with_markdown_editor("old");
    select_all_active_text(&mut app);
    let mut clipboard = TestDocumentClipboard::with_html("<p>different</p>", "plain");
    app.dispatch_document_paste_with_clipboard_for_test(&EditCommand::Paste, &mut clipboard);
    assert_eq!(active_text(&app), "plain");
    let revision_after_paste = active_content_revision(&app);
    dispatch_undo(&mut app);
    assert_eq!(active_text(&app), "old");
    dispatch_redo(&mut app);
    assert_eq!(active_text(&app), "plain");
    assert!(active_content_revision(&app) > revision_after_paste);
}
```

Define the setup helpers in the same test module:

```rust
fn app_with_text(text: &str) -> App {
    let mut app = App::new(None);
    app.push_entry_for_test(
        DocumentView::new(vec![text.into()], 40, 40.0),
        Box::new(crate::plugins::editor::EditorPlugin::new()),
    );
    app.switch_workspace_for_test(0);
    app
}

#[cfg(feature = "markdown")]
fn app_with_markdown_editor(text: &str) -> App {
    let mut app = App::new(None);
    app.push_entry_for_test(
        DocumentView::new(vec![text.into()], 40, 40.0),
        Box::new(textora_markdown::view::MarkdownEditorView::new()),
    );
    app.switch_workspace_for_test(0);
    app
}

fn set_active_selection(app: &mut App, anchor: usize, cursor: usize) {
    let mut tab = app.active_tab_session_mut().expect("active tab should exist");
    tab.document.cursor_move_to_offset(cursor);
    tab.document.cursor_mut().selection_anchor = Some(anchor);
}

fn active_cursor(app: &App) -> usize {
    app.active_tab_session().expect("active tab should exist").document.cursor_offset().to_usize()
}

fn active_selection(app: &App) -> Option<(usize, usize)> {
    app.active_tab_session().expect("active tab should exist").document.selection_range()
}

#[derive(Debug, PartialEq, Eq)]
struct ActiveDocumentSnapshot {
    text: String,
    cursor: usize,
    selection: Option<(usize, usize)>,
    content_revision: u64,
    dirty: bool,
}

fn active_document_snapshot(app: &App) -> ActiveDocumentSnapshot {
    let tab = app.active_tab_session().expect("active tab should exist");
    ActiveDocumentSnapshot {
        text: tab.document.full_text(),
        cursor: tab.document.cursor_offset().to_usize(),
        selection: tab.document.selection_range(),
        content_revision: tab.document.content_revision(),
        dirty: tab.document.dirty,
    }
}

fn active_content_revision(app: &App) -> u64 {
    app.active_tab_session().expect("active tab should exist").document.content_revision()
}

fn select_all_active_text(app: &mut App) {
    app.active_tab_session_mut().expect("active tab should exist").document.select_all();
}
```

Run: `cargo test -p textora-app --lib clipboard::tests`

Expected: PASS.

Run: `cargo test -p textora-app --lib dispatch::editor::edit_tests`

Expected: PASS.

Run: `cargo check -p textora-app --no-default-features`

Expected: PASS; plain editing builds without `textora-markdown`.

- [ ] **Step 7: Commit app orchestration**

```bash
git add crates/app/src/clipboard.rs crates/app/src/commands.rs crates/app/src/dispatch/editor.rs
git commit -m "feat(app): route smart paste through edit transactions"
```

---

### Task 14: Add conversion performance benchmarks

**Files:**
- Create: `crates/markdown/benches/paste_conversion.rs`
- Modify: `crates/markdown/Cargo.toml`

**Interfaces:**
- Consumes: public `prepare_paste`.
- Produces: Criterion benchmark target `paste_conversion`.

- [ ] **Step 1: Add the explicit bench target**

```toml
[[bench]]
name = "paste_conversion"
harness = false
```

- [ ] **Step 2: Implement deterministic generated fixtures**

The benchmark file must generate, without filesystem or network I/O:

```rust
fn paragraph_html(count: usize) -> (String, String);
fn nested_list_html(depth: usize, breadth: usize) -> (String, String);
fn table_html(rows: usize, columns: usize) -> (String, String);
fn office_span_html(count: usize) -> (String, String);
fn code_block_html(lines: usize) -> (String, String);
fn rtf_paragraphs(count: usize) -> (Vec<u8>, String);
```

Each function returns rich input and matching plain text. `code_block_html` must include repeated indentation and internal backtick runs so both preformatted equivalence and fence selection execute. Benchmark `prepare_paste` with `black_box` for all six fixtures; assert the result is not `Empty` before timing.

- [ ] **Step 3: Compile and run a smoke benchmark**

Run: `cargo bench -p textora-markdown --bench paste_conversion -- --test`

Expected: PASS and Criterion reports all six benchmark functions without running a long measurement session.

- [ ] **Step 4: Commit benchmarks**

```bash
git add crates/markdown/Cargo.toml crates/markdown/benches/paste_conversion.rs
git commit -m "perf(markdown): benchmark rich paste conversion"
```

---

### Task 15: Document manual interoperability checks and verify everything

**Files:**
- Modify: `docs/manual_test_protocol.md`

**Interfaces:**
- Produces: repeatable browser/Office/Markdown-editor acceptance matrix.

- [ ] **Step 1: Add a smart paste manual test section**

Document these exact rows with source, action and expected result:

| Source | Action | Expected |
|---|---|---|
| Safari/Chrome article | Copy heading, paragraphs, bold, list, link and remote image; paste with Cmd+V in WYSIWYG | Markdown source contains heading/list/style/link/image syntax; visible order matches source |
| Word/Pages/Feishu | Copy paragraphs, inline styles and list; paste with Cmd+V in WYSIWYG | HTML is preferred; if absent, supported RTF semantics survive |
| VS Code Markdown source | Copy syntax-highlighted source; paste with Cmd+V in WYSIWYG | Raw Markdown source is not converted a second time |
| Typora rendered content | Copy rendered heading/list/link; paste with Cmd+V | Semantic HTML converts to Markdown |
| Any rich source | Paste with Cmd+V in Markdown source view and `.txt` | Only plain text appears; original line breaks remain |
| Any rich source | Paste with Cmd+Shift+V in WYSIWYG | Only `text/plain` is used |
| Remote/embedded image mix | Paste with Cmd+V | HTTP(S) image remains; data/file/cid image is not saved or fetched |
| Selected text | Paste then Undo/Redo | Paste replaces once; one Undo restores selection content; one Redo reapplies |

Also record Linux desktop/session type during tests; distinguish X11 success from unverified pure Wayland behavior.

- [ ] **Step 2: Run focused test suites**

Run:

```bash
cargo test -p textora-appkit-shell --lib clipboard::tests
cargo test -p textora-markdown --lib paste::
cargo test -p textora-app --lib clipboard::tests
cargo test -p textora-app --lib dispatch::editor::edit_tests
cargo check -p textora-app --no-default-features
```

Expected: every command exits 0 with zero failed tests.

- [ ] **Step 3: Run formatting and architecture checks**

Run:

```bash
cargo fmt --all -- --check
./scripts/check_architecture.sh
```

Expected: both commands exit 0.

- [ ] **Step 4: Run the required comprehensive verification**

Run: `./scripts/verify.sh`

Expected: exit 0. Read the full output and report exact failing command if it does not pass; do not claim completion from partial checks.

- [ ] **Step 5: Confirm only intended files changed**

Run: `git status --short`

Expected: only `docs/manual_test_protocol.md` remains uncommitted at this task boundary. Existing user-owned changes present before execution must remain untouched and be listed separately.

- [ ] **Step 6: Commit the acceptance protocol**

```bash
git add docs/manual_test_protocol.md
git commit -m "docs: add smart paste acceptance protocol"
```

---

## Plan Self-Review Checklist

- Every design requirement maps to at least one task: platform snapshot (1–3), converter and safety (4–9), mode policy (10–13), performance (14), manual and comprehensive verification (15).
- No task changes more than three listed files.
- Public type and function names are consistent across producers and consumers.
- Plain-text paths never construct a rich snapshot.
- Explicit Markdown bypasses visible-text comparison; HTML/RTF never bypass it when plain text exists.
- Undo isolation is enforced before and after the `InsertText` transaction.
- No image persistence, network I/O, source-application heuristic or async generation race is introduced.
- Pure Wayland rich clipboard support is not claimed without platform evidence.
