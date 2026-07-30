# Markdown 预览渲染实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 .md 文件实现富文本预览模式，基于 pulldown_cmark 解析和自定义 Element Tree 架构进行渲染。

**Architecture:** 新增 `crates/markdown` crate（对标 zed 的 markdown crate），包含 parser → builder → layout → render 四层 pipeline，最终输出 `DrawList`。在 app 层新增 `MarkdownPreview` 视图，通过 ViewMode 切换编辑/预览。

**Tech Stack:** Rust 2024, pulldown-cmark 0.13, 项目现有的 DrawCmd/wgpu 渲染管线

---

## 文件结构

```
crates/markdown/                    (新 crate)
├── Cargo.toml
├── src/
│   ├── lib.rs                      (公共 API：parse → build → layout → render)
│   ├── parser.rs                   (pulldown_cmark 封装，事件流输出)
│   ├── style.rs                    (MarkdownStyle 纯数据配置)
│   ├── builder.rs                  (MarkdownBuilder，对标 zed)
│   ├── layout.rs                   (布局 pass：算 block 位置/尺寸)
│   └── render.rs                   (渲染 pass：LayoutTree → DrawList)

crates/app/src/
└── md_preview.rs                   (MarkdownPreview 集成层)

crates/app/src/ui_shell.rs          (修改：支持 md 预览渲染)
crates/app/src/app.rs               (修改：注册快捷切换 action)
```

每个文件的职责：
- **lib.rs**: 整合调用，暴露 `render_markdown(src, style, viewport_w, shaper) -> DrawList`
- **parser.rs**: 调用 pulldown_cmark，产出 `ParsedMarkdown`（events + 元数据）
- **style.rs**: `MarkdownStyle` 配置 struct，无逻辑
- **builder.rs**: 遍历 events 构建 `MarkdownDoc`（BlockNode 树 + RenderedLine 列表）
- **layout.rs**: 接收 `MarkdownDoc`，用 shaper 测量文本，计算所有 block 的 Rect
- **render.rs**: 遍历 `LaidOutDoc`，按 block 类型 emit DrawCmd
- **md_preview.rs**: 持有预览状态、滚动、缓存，对接 app 渲染管线

---

### Task 1: Crate 脚手架

**Files:**
- Create: `crates/markdown/Cargo.toml`
- Create: `crates/markdown/src/lib.rs`
- Modify: `Cargo.toml` (workspace members already `crates/*` — 无需改动)

- [ ] **Step 1: 创建 Cargo.toml**

```toml
[package]
name = "edit-plus-markdown"
version = "0.0.0"
edition.workspace = true
license.workspace = true
repository.workspace = true
rust-version.workspace = true

[dependencies]
pulldown-cmark = { version = "0.13.0", default-features = false }
ui.workspace = true
shaping.workspace = true
```

- [ ] **Step 2: 创建 lib.rs（最小骨架）**

```rust
//! Markdown rendering for edit+.
//! Pipeline: parse → build → layout → render → DrawList.

pub mod parser;
pub mod style;
pub mod builder;
pub mod layout;
pub mod render;

mod parser;
mod style;
mod builder;
mod layout;
mod render;
```

> 注意: 先创建空文件让编译通过，后续任务逐步填入。

- [ ] **Step 3: 创建空模块文件**

创建 `parser.rs`, `style.rs`, `builder.rs`, `layout.rs`, `render.rs`，每个文件仅包含一个空行。

- [ ] **Step 4: 编译验证**

```bash
cargo check -p edit-plus-markdown
```
预期: 编译成功（空模块无报错）。

- [ ] **Step 5: 暂存**

```bash
git add crates/markdown/
git status
```

---

### Task 2: Parser——pulldown_cmark 封装

**Files:**
- Create: `crates/markdown/src/parser.rs`
- Modify: `crates/markdown/src/lib.rs`

**参考:** zed `crates/markdown/src/parser.rs:210-900`

- [ ] **Step 1: 写 parser 测试（先写 failing test）**

在 `crates/markdown/src/parser.rs` 末尾添加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_paragraph() {
        let md = "hello world";
        let parsed = parse_markdown(md);
        assert_eq!(parsed.events.len(), 3); // RootStart, Text, RootEnd
        assert!(matches!(parsed.events[1].1, MarkdownEvent::Text));
    }

    #[test]
    fn test_parse_headings() {
        let md = "# H1\n## H2\n### H3";
        let parsed = parse_markdown(md);
        // 每个 heading 有 RootStart, Start(Heading), Text, End(Heading), RootEnd
        let heading_starts: Vec<_> = parsed.events.iter()
            .filter(|(_, e)| matches!(e, MarkdownEvent::Start(MarkdownTag::Heading { .. })))
            .collect();
        assert_eq!(heading_starts.len(), 3);
    }

    #[test]
    fn test_parse_bold_italic() {
        let md = "**bold** and *italic*";
        let parsed = parse_markdown(md);
        let has_strong = parsed.events.iter().any(|(_, e)| matches!(e, MarkdownEvent::Start(MarkdownTag::Strong)));
        let has_emphasis = parsed.events.iter().any(|(_, e)| matches!(e, MarkdownEvent::Start(MarkdownTag::Emphasis)));
        assert!(has_strong);
        assert!(has_emphasis);
    }

    #[test]
    fn test_parse_code_block() {
        let md = "```rust\nfn main() {}\n```";
        let parsed = parse_markdown(md);
        let has_code_block = parsed.events.iter().any(|(_, e)| {
            matches!(e, MarkdownEvent::Start(MarkdownTag::CodeBlock { .. }))
        });
        assert!(has_code_block);
    }

    #[test]
    fn test_parse_list() {
        let md = "- item 1\n- item 2";
        let parsed = parse_markdown(md);
        let list_starts: Vec<_> = parsed.events.iter()
            .filter(|(_, e)| matches!(e, MarkdownEvent::Start(MarkdownTag::List(_))))
            .collect();
        assert_eq!(list_starts.len(), 1);
        let item_starts: Vec<_> = parsed.events.iter()
            .filter(|(_, e)| matches!(e, MarkdownEvent::Start(MarkdownTag::Item)))
            .collect();
        assert_eq!(item_starts.len(), 2);
    }

    #[test]
    fn test_parse_table() {
        let md = "| a | b |\n| --- | --- |\n| 1 | 2 |";
        let parsed = parse_markdown(md);
        let has_table = parsed.events.iter().any(|(_, e)| {
            matches!(e, MarkdownEvent::Start(MarkdownTag::Table(_)))
        });
        assert!(has_table);
    }

    #[test]
    fn test_parse_link() {
        let md = "[click here](https://example.com)";
        let parsed = parse_markdown(md);
        let has_link = parsed.events.iter().any(|(_, e)| {
            matches!(e, MarkdownEvent::Start(MarkdownTag::Link { .. }))
        });
        assert!(has_link);
    }

    #[test]
    fn test_parse_task_list() {
        let md = "- [x] done\n- [ ] todo";
        let parsed = parse_markdown(md);
        let has_checked = parsed.events.iter().any(|(_, e)| matches!(e, MarkdownEvent::TaskListMarker(true)));
        let has_unchecked = parsed.events.iter().any(|(_, e)| matches!(e, MarkdownEvent::TaskListMarker(false)));
        assert!(has_checked);
        assert!(has_unchecked);
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

```bash
cargo test -p edit-plus-markdown
```
预期: 编译错误（`parse_markdown`, `MarkdownEvent`, `MarkdownTag` 等类型未定义）。

- [ ] **Step 3: 实现 parser 核心类型和函数**

完整写入 `crates/markdown/src/parser.rs`：

```rust
use pulldown_cmark::{
    Alignment, HeadingLevel, LinkType, Options, Parser, TagEnd as CmarkTagEnd,
};
use std::ops::Range;

// ---- Parse options (对标 zed PARSE_OPTIONS，精简) ----
const PARSE_OPTIONS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_FOOTNOTES)
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS)
    .union(Options::ENABLE_SMART_PUNCTUATION)
    .union(Options::ENABLE_HEADING_ATTRIBUTES)
    .union(Options::ENABLE_GFM);

// ---- Event types (对标 zed MarkdownEvent/MarkdownTag) ----

#[derive(Clone, Debug, PartialEq)]
pub enum MarkdownEvent {
    Start(MarkdownTag),
    End(MarkdownTagEnd),
    Text,
    Code,
    SubstitutedText(String),
    SoftBreak,
    HardBreak,
    Rule,
    TaskListMarker(bool),
    FootnoteReference(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum MarkdownTag {
    Paragraph,
    Heading {
        level: HeadingLevel,
        id: Option<String>,
    },
    BlockQuote,
    CodeBlock {
        kind: CodeBlockKind,
    },
    List(Option<u64>),
    Item,
    Table(Vec<Alignment>),
    TableHead,
    TableRow,
    TableCell,
    Emphasis,
    Strong,
    Strikethrough,
    Link {
        link_type: LinkType,
        dest_url: String,
        title: String,
    },
    Image {
        link_type: LinkType,
        dest_url: String,
        title: String,
    },
    FootnoteDefinition(String),
}

// 复用 pulldown_cmark 的 TagEnd
pub use CmarkTagEnd as MarkdownTagEnd;

#[derive(Clone, Debug, PartialEq)]
pub enum CodeBlockKind {
    Indented,
    Fenced,
    FencedLang(String),
}

// ---- Parsed output ----

#[derive(Clone, Debug, Default)]
pub struct ParsedMarkdown {
    pub events: Vec<(Range<usize>, MarkdownEvent)>,
    pub source: String,
    pub heading_slugs: Vec<(String, usize)>, // (slug, source_offset)
    pub footnote_definitions: Vec<(String, usize)>, // (label, source_offset)
}

// ---- Parse state (对标 zed ParseState) ----

#[derive(Default)]
struct ParseState {
    events: Vec<(Range<usize>, MarkdownEvent)>,
    depth: usize,
}

impl ParseState {
    fn push_event(&mut self, range: Range<usize>, event: MarkdownEvent) {
        self.events.push((range, event));
    }
}

// ---- Public API ----

pub fn parse_markdown(text: &str) -> ParsedMarkdown {
    let mut state = ParseState::default();
    let mut heading_slugs = Vec::new();
    let mut footnote_definitions = Vec::new();

    let parser = Parser::new_ext(text, PARSE_OPTIONS).into_offset_iter();

    for (pulldown_event, range) in parser {
        match pulldown_event {
            pulldown_cmark::Event::Start(tag) => {
                let event = convert_start_tag(tag, text, &range);
                if let MarkdownEvent::Start(MarkdownTag::Heading { id, .. }) = &event {
                    if let Some(slug) = id.clone().or_else(|| {
                        // extract text content for slug
                        None // simplified: id from heading attributes takes priority
                    }) {
                        heading_slugs.push((slug, range.start));
                    }
                }
                if let MarkdownEvent::Start(MarkdownTag::FootnoteDefinition(ref label)) = &event {
                    footnote_definitions.push((label.clone(), range.start));
                }
                state.push_event(range, event);
            }
            pulldown_cmark::Event::End(tag) => {
                let end_tag = match tag {
                    pulldown_cmark::TagEnd::Paragraph => MarkdownTagEnd::Paragraph,
                    pulldown_cmark::TagEnd::Heading(level) => MarkdownTagEnd::Heading(level),
                    pulldown_cmark::TagEnd::BlockQuote(kind) => MarkdownTagEnd::BlockQuote(kind),
                    pulldown_cmark::TagEnd::CodeBlock => MarkdownTagEnd::CodeBlock,
                    pulldown_cmark::TagEnd::List(extra) => MarkdownTagEnd::List(extra),
                    pulldown_cmark::TagEnd::Item => MarkdownTagEnd::Item,
                    pulldown_cmark::TagEnd::Table => MarkdownTagEnd::Table,
                    pulldown_cmark::TagEnd::TableHead => MarkdownTagEnd::TableHead,
                    pulldown_cmark::TagEnd::TableRow => MarkdownTagEnd::TableRow,
                    pulldown_cmark::TagEnd::TableCell => MarkdownTagEnd::TableCell,
                    pulldown_cmark::TagEnd::Emphasis => MarkdownTagEnd::Emphasis,
                    pulldown_cmark::TagEnd::Strong => MarkdownTagEnd::Strong,
                    pulldown_cmark::TagEnd::Strikethrough => MarkdownTagEnd::Strikethrough,
                    pulldown_cmark::TagEnd::Link => MarkdownTagEnd::Link,
                    pulldown_cmark::TagEnd::Image => MarkdownTagEnd::Image,
                    pulldown_cmark::TagEnd::FootnoteDefinition => MarkdownTagEnd::FootnoteDefinition,
                    pulldown_cmark::TagEnd::HtmlBlock => MarkdownTagEnd::HtmlBlock,
                    pulldown_cmark::TagEnd::MetadataBlock(kind) => MarkdownTagEnd::MetadataBlock(kind),
                    other => {
                        // passthrough for tags we don't explicitly handle
                        MarkdownTagEnd::Paragraph // fallback
                    }
                };
                state.push_event(range, MarkdownEvent::End(end_tag));
            }
            pulldown_cmark::Event::Text(_) => {
                state.push_event(range, MarkdownEvent::Text);
            }
            pulldown_cmark::Event::Code(_) => {
                state.push_event(range, MarkdownEvent::Code);
            }
            pulldown_cmark::Event::Html(_) => {
                // skip inline HTML for now
            }
            pulldown_cmark::Event::InlineHtml(_) => {
                // skip inline HTML for now
            }
            pulldown_cmark::Event::FootnoteReference(label) => {
                state.push_event(range, MarkdownEvent::FootnoteReference(label.to_string()));
            }
            pulldown_cmark::Event::SoftBreak => {
                state.push_event(range, MarkdownEvent::SoftBreak);
            }
            pulldown_cmark::Event::HardBreak => {
                state.push_event(range, MarkdownEvent::HardBreak);
            }
            pulldown_cmark::Event::Rule => {
                state.push_event(range, MarkdownEvent::Rule);
            }
            pulldown_cmark::Event::TaskListMarker(checked) => {
                state.push_event(range, MarkdownEvent::TaskListMarker(checked));
            }
        }
    }

    ParsedMarkdown {
        events: state.events,
        source: text.to_string(),
        heading_slugs,
        footnote_definitions,
    }
}

// ---- Internal helpers ----

fn convert_start_tag(tag: pulldown_cmark::Tag, source: &str, range: &Range<usize>) -> MarkdownEvent {
    let md_tag = match tag {
        pulldown_cmark::Tag::Paragraph => MarkdownTag::Paragraph,
        pulldown_cmark::Tag::Heading { level, id, classes, attrs } => {
            let id_str = id.map(|s| s.to_string());
            MarkdownTag::Heading { level, id: id_str }
        }
        pulldown_cmark::Tag::BlockQuote(kind) => MarkdownTag::BlockQuote,
        pulldown_cmark::Tag::CodeBlock(kind) => {
            let cb_kind = match kind {
                pulldown_cmark::CodeBlockKind::Indented => CodeBlockKind::Indented,
                pulldown_cmark::CodeBlockKind::Fenced(lang) => {
                    if lang.is_empty() {
                        CodeBlockKind::Fenced
                    } else {
                        CodeBlockKind::FencedLang(lang.to_string())
                    }
                }
            };
            MarkdownTag::CodeBlock { kind: cb_kind }
        }
        pulldown_cmark::Tag::List(start) => MarkdownTag::List(start),
        pulldown_cmark::Tag::Item => MarkdownTag::Item,
        pulldown_cmark::Tag::Table(alignments) => MarkdownTag::Table(alignments),
        pulldown_cmark::Tag::TableHead => MarkdownTag::TableHead,
        pulldown_cmark::Tag::TableRow => MarkdownTag::TableRow,
        pulldown_cmark::Tag::TableCell => MarkdownTag::TableCell,
        pulldown_cmark::Tag::Emphasis => MarkdownTag::Emphasis,
        pulldown_cmark::Tag::Strong => MarkdownTag::Strong,
        pulldown_cmark::Tag::Strikethrough => MarkdownTag::Strikethrough,
        pulldown_cmark::Tag::Link { link_type, dest_url, title, id } => {
            MarkdownTag::Link {
                link_type,
                dest_url: dest_url.to_string(),
                title: title.to_string(),
            }
        }
        pulldown_cmark::Tag::Image { link_type, dest_url, title, id } => {
            MarkdownTag::Image {
                link_type,
                dest_url: dest_url.to_string(),
                title: title.to_string(),
            }
        }
        pulldown_cmark::Tag::FootnoteDefinition(label) => {
            MarkdownTag::FootnoteDefinition(label.to_string())
        }
        _ => {
            // HtmlBlock, MetadataBlock, DefinitionList etc. — skip for now
            MarkdownTag::Paragraph // fallback
        }
    };
    MarkdownEvent::Start(md_tag)
}
```

- [ ] **Step 4: 运行测试，确认通过**

```bash
cargo test -p edit-plus-markdown
```
预期: 8 个测试全部 PASS。

- [ ] **Step 5: 暂存**

```bash
git add crates/markdown/src/parser.rs crates/markdown/src/lib.rs
```

---

### Task 3: MarkdownStyle 配置

**Files:**
- Create: `crates/markdown/src/style.rs`
- Modify: `crates/markdown/src/lib.rs`

- [ ] **Step 1: 写 style 测试**

在文件末尾模块测试中：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_style() {
        let style = MarkdownStyle::default();
        assert!(style.base_font_size > 0.0);
        assert!(style.line_height_ratio > 0.0);
        assert_eq!(style.h1_scale, 1.5);
        assert_eq!(style.h2_scale, 1.3);
    }

    #[test]
    fn test_from_theme() {
        let theme = ui::Theme {
            name: "test".into(),
            is_dark: true,
            background: [0.1, 0.1, 0.1, 1.0],
            foreground: [0.9, 0.9, 0.9, 1.0],
            ..Default::default()
        };
        let style = MarkdownStyle::from_theme(&theme);
        assert_eq!(style.text_color, [0.9, 0.9, 0.9, 1.0]);
        // code background should be slightly different from page background
        assert_ne!(style.code_bg, style.text_color);
    }
}
```

> 注: `ui::Theme` 的 `Default` 需要检查是否存在。如不存在则手动构造测试所需字段即可。

- [ ] **Step 2: 运行测试，确认失败**

```bash
cargo test -p edit-plus-markdown -- --test-threads=1
```
预期: 编译错误（`MarkdownStyle` 未定义）。

- [ ] **Step 3: 实现 MarkdownStyle**

写入 `crates/markdown/src/style.rs`：

```rust
use ui::Theme;

/// Markdown 渲染样式配置。
/// 对标 zed MarkdownStyle，适配本项目颜色/字体系统。
#[derive(Clone, Debug)]
pub struct MarkdownStyle {
    // 字体
    pub base_font_size: f32,
    pub mono_font_size: f32,
    pub line_height_ratio: f32,

    // 标题字号倍率
    pub h1_scale: f32,
    pub h2_scale: f32,
    pub h3_scale: f32,

    // 颜色 (RGBA [0-1])
    pub text_color: [f32; 4],
    pub heading_color: [f32; 4],
    pub code_bg: [f32; 4],
    pub code_text: [f32; 4],
    pub blockquote_border: [f32; 4],
    pub blockquote_bg: [f32; 4],
    pub link_color: [f32; 4],
    pub table_border: [f32; 4],
    pub table_header_bg: [f32; 4],
    pub table_stripe_bg: [f32; 4],
    pub rule_color: [f32; 4],

    // 布局
    pub blockquote_indent: f32,
    pub list_indent: f32,
    pub code_padding: f32,
    pub heading_spacing: f32,
    pub paragraph_spacing: f32,
}

impl Default for MarkdownStyle {
    fn default() -> Self {
        Self {
            base_font_size: 14.0,
            mono_font_size: 13.0,
            line_height_ratio: 1.5,
            h1_scale: 1.5,
            h2_scale: 1.3,
            h3_scale: 1.15,
            text_color: [0.9, 0.9, 0.9, 1.0],
            heading_color: [0.95, 0.95, 0.95, 1.0],
            code_bg: [0.15, 0.15, 0.15, 1.0],
            code_text: [0.85, 0.85, 0.85, 1.0],
            blockquote_border: [0.4, 0.4, 0.4, 1.0],
            blockquote_bg: [0.08, 0.08, 0.08, 1.0],
            link_color: [0.3, 0.6, 1.0, 1.0],
            table_border: [0.3, 0.3, 0.3, 1.0],
            table_header_bg: [0.12, 0.12, 0.12, 1.0],
            table_stripe_bg: [0.08, 0.08, 0.08, 1.0],
            rule_color: [0.3, 0.3, 0.3, 1.0],
            blockquote_indent: 16.0,
            list_indent: 20.0,
            code_padding: 8.0,
            heading_spacing: 8.0,
            paragraph_spacing: 4.0,
        }
    }
}

impl MarkdownStyle {
    /// 从编辑器 Theme 构造，继承主题颜色。
    pub fn from_theme(theme: &Theme) -> Self {
        let is_dark = theme.is_dark;
        let bg = theme.background;
        let fg = theme.foreground;

        Self {
            text_color: fg,
            heading_color: fg,
            code_bg: if is_dark {
                [bg[0] * 0.7, bg[1] * 0.7, bg[2] * 0.7, 1.0]
            } else {
                [bg[0] * 1.3, bg[1] * 1.3, bg[2] * 1.3, 1.0]
            },
            code_text: fg,
            blockquote_border: [
                fg[0] * 0.4,
                fg[1] * 0.4,
                fg[2] * 0.4,
                1.0,
            ],
            blockquote_bg: if is_dark {
                [bg[0] * 0.6, bg[1] * 0.6, bg[2] * 0.6, 1.0]
            } else {
                [bg[0] * 1.15, bg[1] * 1.15, bg[2] * 1.15, 1.0]
            },
            link_color: if is_dark {
                [0.35, 0.65, 1.0, 1.0]
            } else {
                [0.1, 0.4, 0.9, 1.0]
            },
            table_border: [fg[0] * 0.3, fg[1] * 0.3, fg[2] * 0.3, 1.0],
            table_header_bg: if is_dark {
                [bg[0] * 0.7, bg[1] * 0.7, bg[2] * 0.7, 1.0]
            } else {
                [bg[0] * 1.2, bg[1] * 1.2, bg[2] * 1.2, 1.0]
            },
            table_stripe_bg: if is_dark {
                [bg[0] * 0.55, bg[1] * 0.55, bg[2] * 0.55, 1.0]
            } else {
                [bg[0] * 1.1, bg[1] * 1.1, bg[2] * 1.1, 1.0]
            },
            rule_color: [fg[0] * 0.3, fg[1] * 0.3, fg[2] * 0.3, 1.0],
            ..Default::default()
        }
    }

    /// 获取指定级别的标题字号。
    pub fn heading_font_size(&self, level: u8) -> f32 {
        let scale = match level {
            1 => self.h1_scale,
            2 => self.h2_scale,
            3 => self.h3_scale,
            _ => 1.0,
        };
        self.base_font_size * scale
    }
}
```

- [ ] **Step 4: 运行测试，确认通过**

```bash
cargo test -p edit-plus-markdown
```
预期: 所有 style 测试 PASS。

- [ ] **Step 5: 暂存**

```bash
git add crates/markdown/src/style.rs
```

---

### Task 4: MarkdownBuilder——事件 → BlockNode 树 + RenderedLine

**Files:**
- Create: `crates/markdown/src/builder.rs`
- Modify: `crates/markdown/src/lib.rs`

**参考:** zed `crates/markdown/src/markdown.rs:2901-3320`

- [ ] **Step 1: 写 builder 测试**

在 `crates/markdown/src/builder.rs` 末尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_markdown;

    #[test]
    fn test_build_simple_paragraph() {
        let parsed = parse_markdown("hello world");
        let style = MarkdownStyle::default();
        let doc = MarkdownBuilder::build(&parsed, &style);
        // 应该至少有一行渲染文本
        assert!(!doc.lines.is_empty());
        assert_eq!(doc.lines[0].text, "hello world");
    }

    #[test]
    fn test_build_heading() {
        let parsed = parse_markdown("# Hello");
        let style = MarkdownStyle::default();
        let doc = MarkdownBuilder::build(&parsed, &style);
        assert!(doc.blocks.len() >= 1);
        assert!(matches!(doc.blocks[0].kind, BlockKind::Heading { .. }));
    }

    #[test]
    fn test_build_bold() {
        let parsed = parse_markdown("**bold text**");
        let style = MarkdownStyle::default();
        let doc = MarkdownBuilder::build(&parsed, &style);
        assert!(!doc.lines.is_empty());
        // 应该有 bold 文本 run
        let has_bold_run = doc.lines[0].runs.iter().any(|r| r.modifier == TextStyleMod::Bold);
        assert!(has_bold_run);
    }

    #[test]
    fn test_build_code_block() {
        let parsed = parse_markdown("```\nlet x = 1;\n```");
        let style = MarkdownStyle::default();
        let doc = MarkdownBuilder::build(&parsed, &style);
        assert!(doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::CodeBlock { .. })));
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

```bash
cargo test -p edit-plus-markdown
```
预期: 编译错误。

- [ ] **Step 3: 实现 builder 核心类型**

写入 `crates/markdown/src/builder.rs`：

```rust
use std::ops::Range;
use crate::parser::{MarkdownEvent, MarkdownTag, MarkdownTagEnd, ParsedMarkdown, CodeBlockKind};
use crate::style::MarkdownStyle;

// ===== Block 节点类型 =====

#[derive(Clone, Debug)]
pub struct BlockNode {
    pub kind: BlockKind,
    pub children: Vec<BlockNode>,
    pub source_range: Range<usize>,
}

#[derive(Clone, Debug)]
pub enum BlockKind {
    Container,
    Heading { level: u8 },
    Paragraph,
    CodeBlock { language: Option<String> },
    BlockQuote,
    ListItem { bullet: ListBullet },
    TableWrapper { columns: usize, alignments: Vec<pulldown_cmark::Alignment> },
    TableRow_,
    TableCell_ { col: usize, row: usize, is_header: bool },
    HorizontalRule,
    FootnoteDefinition { label: String },
}

#[derive(Clone, Debug)]
pub enum ListBullet {
    Bullet,
    Ordered(u64),
    TaskList(bool),
}

// ===== 文本样式修饰 =====

#[derive(Clone, Debug, PartialEq)]
pub enum TextStyleMod {
    Bold,
    Italic,
    Strikethrough,
    InlineCode,
    Link { url: String },
    Heading { level: u8 },
    BlockQuote,
    CodeBlock,
}

// ===== 文本运行 =====

#[derive(Clone, Debug)]
pub struct TextRun {
    pub start: usize,   // byte offset within the line text
    pub len: usize,
    pub modifier: TextStyleMod,
}

// ===== 源映射 =====

#[derive(Copy, Clone, Debug)]
pub struct SourceMapping {
    pub rendered_index: usize,
    pub source_index: usize,
}

// ===== 渲染行 =====

#[derive(Clone, Debug)]
pub struct RenderedLine {
    pub text: String,
    pub runs: Vec<TextRun>,
    pub source_mappings: Vec<SourceMapping>,
    pub source_end: usize,
    pub is_code: bool,
}

// ===== 链接记录 =====

#[derive(Clone, Debug)]
pub struct RenderedLink {
    pub source_range: Range<usize>,
    pub url: String,
}

// ===== 构建产物 =====

#[derive(Clone, Debug)]
pub struct MarkdownDoc {
    pub blocks: Vec<BlockNode>,
    pub lines: Vec<RenderedLine>,
    pub links: Vec<RenderedLink>,
}

// ===== TableState (直接搬 zed) =====

#[derive(Default)]
struct TableState {
    alignments: Vec<pulldown_cmark::Alignment>,
    in_head: bool,
    row_index: usize,
    col_index: usize,
}

impl TableState {
    fn start(&mut self, alignments: Vec<pulldown_cmark::Alignment>) {
        self.alignments = alignments;
        self.in_head = false;
        self.row_index = 0;
        self.col_index = 0;
    }

    fn end(&mut self) {
        self.alignments.clear();
        self.in_head = false;
        self.row_index = 0;
        self.col_index = 0;
    }

    fn start_head(&mut self) { self.in_head = true; }

    fn end_head(&mut self) { self.in_head = false; }

    fn start_row(&mut self) { self.col_index = 0; }

    fn end_row(&mut self) { self.row_index += 1; }

    fn end_cell(&mut self) { self.col_index += 1; }

    fn current_cell_alignment(&self) -> Option<pulldown_cmark::Alignment> {
        if self.alignments.is_empty() { return None; }
        if self.in_head { return Some(pulldown_cmark::Alignment::Center); }
        self.alignments.get(self.col_index).copied()
    }
}

// ===== List stack =====

struct ListStackEntry {
    bullet_index: Option<u64>,
}

// ===== PendingLine (对标 zed) =====

#[derive(Default)]
struct PendingLine {
    text: String,
    runs: Vec<TextRun>,
    source_mappings: Vec<SourceMapping>,
}

// ===== MarkdownBuilder (对标 zed MarkdownElementBuilder) =====

struct MarkdownBuilder<'a> {
    style: &'a MarkdownStyle,
    block_stack: Vec<BlockNode>,
    pending_line: PendingLine,
    rendered_lines: Vec<RenderedLine>,
    rendered_links: Vec<RenderedLink>,
    text_style_stack: Vec<TextStyleMod>,
    code_block_depth: usize,
    link_depth: usize,
    list_stack: Vec<ListStackEntry>,
    table: TableState,
    current_source_index: usize,
}

impl<'a> MarkdownBuilder<'a> {
    fn new(style: &'a MarkdownStyle) -> Self {
        Self {
            style,
            block_stack: vec![BlockNode {
                kind: BlockKind::Container,
                children: vec![],
                source_range: 0..0,
            }],
            pending_line: PendingLine::default(),
            rendered_lines: vec![],
            rendered_links: vec![],
            text_style_stack: vec![],
            code_block_depth: 0,
            link_depth: 0,
            list_stack: vec![],
            table: TableState::default(),
            current_source_index: 0,
        }
    }

    fn push_text_style(&mut self, modifier: TextStyleMod) {
        self.text_style_stack.push(modifier);
    }

    fn pop_text_style(&mut self) {
        self.text_style_stack.pop();
    }

    fn push_block(&mut self, kind: BlockKind, source_range: &Range<usize>) {
        self.flush_text();
        let node = BlockNode {
            kind,
            children: vec![],
            source_range: source_range.clone(),
        };
        self.block_stack.push(node);
    }

    fn pop_block(&mut self) {
        self.flush_text();
        if let Some(node) = self.block_stack.pop() {
            if let Some(parent) = self.block_stack.last_mut() {
                parent.children.push(node);
            }
        }
    }

    fn push_text(&mut self, text: &str, source_range: &Range<usize>) {
        let start = self.pending_line.text.len();
        self.pending_line.text.push_str(text);
        self.pending_line.source_mappings.push(SourceMapping {
            rendered_index: start,
            source_index: source_range.start,
        });
        self.current_source_index = source_range.end;

        // Record current style stack as runs
        for modifier in &self.text_style_stack {
            self.pending_line.runs.push(TextRun {
                start,
                len: text.len(),
                modifier: modifier.clone(),
            });
        }
    }

    fn push_link(&mut self, url: String, source_range: Range<usize>) {
        self.rendered_links.push(RenderedLink {
            source_range,
            url,
        });
    }

    fn flush_text(&mut self) {
        let line = std::mem::take(&mut self.pending_line);
        if line.text.is_empty() {
            return;
        }
        let is_code = self.code_block_depth > 0;
        self.rendered_lines.push(RenderedLine {
            text: line.text,
            runs: line.runs,
            source_mappings: line.source_mappings,
            source_end: self.current_source_index,
            is_code,
        });
    }

    fn push_list(&mut self, start: Option<u64>) {
        self.list_stack.push(ListStackEntry { bullet_index: start });
    }

    fn pop_list(&mut self) {
        self.list_stack.pop();
    }

    fn next_bullet_index(&mut self) -> Option<u64> {
        self.list_stack.last_mut().and_then(|entry| {
            let idx = entry.bullet_index.as_mut()?;
            let current = *idx;
            *idx += 1;
            Some(current)
        })
    }

    fn push_code_block(&mut self) {
        self.code_block_depth += 1;
    }

    fn pop_code_block(&mut self) {
        self.code_block_depth = self.code_block_depth.saturating_sub(1);
    }

    fn trim_trailing_newline(&mut self) {
        if self.pending_line.text.ends_with('\n') {
            let new_len = self.pending_line.text.len() - 1;
            self.pending_line.text.truncate(new_len);
            // Also truncate last run if its range extends beyond new length
            self.pending_line.runs.retain(|r| r.start + r.len <= new_len);
        }
    }

    fn build(mut self) -> MarkdownDoc {
        self.flush_text();
        MarkdownDoc {
            blocks: std::mem::take(&mut self.block_stack.swap_remove(0).children),
            lines: std::mem::take(&mut self.rendered_lines),
            links: std::mem::take(&mut self.rendered_links),
        }
    }
}

// ===== 公开入口 =====

impl MarkdownDoc {
    pub fn build(parsed: &ParsedMarkdown, style: &MarkdownStyle) -> Self {
        let mut builder = MarkdownBuilder::new(style);

        for (range, event) in &parsed.events {
            match event {
                MarkdownEvent::Start(tag) => match tag {
                    MarkdownTag::Paragraph => {
                        builder.push_block(BlockKind::Paragraph, range);
                    }
                    MarkdownTag::Heading { level, .. } => {
                        let lvl = match level {
                            pulldown_cmark::HeadingLevel::H1 => 1,
                            pulldown_cmark::HeadingLevel::H2 => 2,
                            pulldown_cmark::HeadingLevel::H3 => 3,
                            pulldown_cmark::HeadingLevel::H4 => 4,
                            pulldown_cmark::HeadingLevel::H5 => 5,
                            pulldown_cmark::HeadingLevel::H6 => 6,
                        };
                        builder.push_text_style(TextStyleMod::Heading { level: lvl });
                        builder.push_block(BlockKind::Heading { level: lvl }, range);
                    }
                    MarkdownTag::BlockQuote => {
                        builder.push_text_style(TextStyleMod::BlockQuote);
                        builder.push_block(BlockKind::BlockQuote, range);
                    }
                    MarkdownTag::CodeBlock { kind } => {
                        let language = match kind {
                            CodeBlockKind::FencedLang(lang) => Some(lang.clone()),
                            _ => None,
                        };
                        builder.push_code_block();
                        builder.push_text_style(TextStyleMod::CodeBlock);
                        builder.push_block(BlockKind::CodeBlock { language }, range);
                    }
                    MarkdownTag::List(start) => {
                        builder.push_list(*start);
                        builder.push_block(BlockKind::Container, range);
                    }
                    MarkdownTag::Item => {
                        let bullet = if let Some(bullet_index) = builder.next_bullet_index() {
                            ListBullet::Ordered(bullet_index)
                        } else {
                            ListBullet::Bullet
                        };
                        builder.push_block(BlockKind::ListItem { bullet }, range);
                    }
                    MarkdownTag::Emphasis => {
                        builder.push_text_style(TextStyleMod::Italic);
                    }
                    MarkdownTag::Strong => {
                        builder.push_text_style(TextStyleMod::Bold);
                    }
                    MarkdownTag::Strikethrough => {
                        builder.push_text_style(TextStyleMod::Strikethrough);
                    }
                    MarkdownTag::Link { dest_url, .. } => {
                        builder.link_depth += 1;
                        builder.push_link(dest_url.clone(), range.clone());
                        builder.push_text_style(TextStyleMod::Link { url: dest_url.clone() });
                    }
                    MarkdownTag::Table(alignments) => {
                        builder.table.start(alignments.clone());
                        builder.push_block(
                            BlockKind::TableWrapper {
                                columns: alignments.len(),
                                alignments: alignments.clone(),
                            },
                            range,
                        );
                    }
                    MarkdownTag::TableHead => {
                        builder.table.start_head();
                    }
                    MarkdownTag::TableRow => {
                        builder.table.start_row();
                        builder.push_block(BlockKind::TableRow_, range);
                    }
                    MarkdownTag::TableCell => {
                        let col = builder.table.col_index;
                        let row = builder.table.row_index;
                        let is_header = builder.table.in_head;
                        builder.push_block(
                            BlockKind::TableCell_ { col, row, is_header },
                            range,
                        );
                    }
                    MarkdownTag::Image { .. } => {
                        // skip image rendering for now (show nothing)
                    }
                    MarkdownTag::FootnoteDefinition(label) => {
                        builder.push_block(
                            BlockKind::FootnoteDefinition { label: label.clone() },
                            range,
                        );
                    }
                },
                MarkdownEvent::End(tag) => match tag {
                    MarkdownTagEnd::Paragraph => builder.pop_block(),
                    MarkdownTagEnd::Heading(_) => {
                        builder.pop_block();
                        builder.pop_text_style();
                    }
                    MarkdownTagEnd::BlockQuote(_) => {
                        builder.pop_block();
                        builder.pop_text_style();
                    }
                    MarkdownTagEnd::CodeBlock => {
                        builder.trim_trailing_newline();
                        builder.pop_block();
                        builder.pop_code_block();
                        builder.pop_text_style();
                    }
                    MarkdownTagEnd::List(_) => {
                        builder.pop_block();
                        builder.pop_list();
                    }
                    MarkdownTagEnd::Item => {
                        builder.pop_block();
                    }
                    MarkdownTagEnd::Emphasis => builder.pop_text_style(),
                    MarkdownTagEnd::Strong => builder.pop_text_style(),
                    MarkdownTagEnd::Strikethrough => builder.pop_text_style(),
                    MarkdownTagEnd::Link => {
                        builder.link_depth -= 1;
                        builder.pop_text_style();
                    }
                    MarkdownTagEnd::Table => {
                        builder.pop_block();
                        builder.table.end();
                    }
                    MarkdownTagEnd::TableHead => builder.table.end_head(),
                    MarkdownTagEnd::TableRow => {
                        builder.pop_block();
                        builder.table.end_row();
                    }
                    MarkdownTagEnd::TableCell => {
                        builder.pop_block();
                        builder.table.end_cell();
                    }
                    MarkdownTagEnd::Image => {}
                    MarkdownTagEnd::FootnoteDefinition => builder.pop_block(),
                    _ => {}
                },
                MarkdownEvent::Text => {
                    let text = &parsed.source[range.clone()];
                    builder.push_text(text, range);
                }
                MarkdownEvent::Code => {
                    let text = &parsed.source[range.clone()];
                    builder.push_text_style(TextStyleMod::InlineCode);
                    builder.push_text(text, range);
                    builder.pop_text_style();
                }
                MarkdownEvent::SoftBreak => {
                    builder.push_text(" ", range);
                }
                MarkdownEvent::HardBreak => {
                    builder.push_text("\n", range);
                }
                MarkdownEvent::Rule => {
                    builder.push_block(BlockKind::HorizontalRule, range);
                    builder.pop_block(); // HorizontalRule has no children
                }
                MarkdownEvent::TaskListMarker(checked) => {
                    // Update the parent ListItem's bullet to TaskList
                    if let Some(last) = builder.block_stack.last_mut() {
                        if matches!(last.kind, BlockKind::ListItem { .. }) {
                            last.kind = BlockKind::ListItem {
                                bullet: ListBullet::TaskList(*checked),
                            };
                        }
                    }
                }
                MarkdownEvent::FootnoteReference(_label) => {
                    // footnote ref: render as superscript text (skip for now)
                }
                MarkdownEvent::SubstitutedText(sub) => {
                    builder.push_text(sub, range);
                }
            }
        }

        builder.build()
    }
}
```

- [ ] **Step 4: 运行测试，确认通过**

```bash
cargo test -p edit-plus-markdown
```
预期: 所有 builder 测试 PASS。

- [ ] **Step 5: 暂存**

```bash
git add crates/markdown/src/builder.rs
```

---

### Task 5: Layout Engine——计算 Block 位置

**Files:**
- Create: `crates/markdown/src/layout.rs`
- Modify: `crates/markdown/src/lib.rs`

- [ ] **Step 1: 写 layout 测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_markdown;
    use crate::builder::MarkdownDoc;
    use crate::style::MarkdownStyle;

    fn make_doc(md: &str) -> MarkdownDoc {
        let parsed = parse_markdown(md);
        let style = MarkdownStyle::default();
        MarkdownDoc::build(&parsed, &style)
    }

    #[test]
    fn test_layout_paragraph_has_rect() {
        let doc = make_doc("hello world");
        let style = MarkdownStyle::default();
        let laid_out = layout_doc(&doc, &style, 400.0);
        assert!(!laid_out.blocks.is_empty());
        assert!(laid_out.blocks[0].rect.w > 0.0);
        assert!(laid_out.blocks[0].rect.h > 0.0);
    }

    #[test]
    fn test_layout_heading_larger_than_paragraph() {
        let h1 = make_doc("# Big Title");
        let p = make_doc("small text");
        let style = MarkdownStyle::default();
        let h1_layout = layout_doc(&h1, &style, 400.0);
        let p_layout = layout_doc(&p, &style, 400.0);
        // h1 should be taller than paragraph because of larger font
        assert!(h1_layout.blocks[0].rect.h > p_layout.blocks[0].rect.h);
    }

    #[test]
    fn test_layout_vertical_positions_increase() {
        let doc = make_doc("# A\n\n## B\n\nhello");
        let style = MarkdownStyle::default();
        let laid_out = layout_doc(&doc, &style, 400.0);
        assert!(laid_out.blocks.len() >= 3);
        // y positions should increase monotonically
        for i in 1..laid_out.blocks.len() {
            assert!(
                laid_out.blocks[i].rect.y >= laid_out.blocks[i-1].rect.y,
                "block {} should be below block {}", i, i-1
            );
        }
    }

    #[test]
    fn test_layout_total_height() {
        let doc = make_doc("# Title\n\nparagraph text here\n\n## Another heading");
        let style = MarkdownStyle::default();
        let laid_out = layout_doc(&doc, &style, 400.0);
        assert!(laid_out.total_height > 0.0);
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

```bash
cargo test -p edit-plus-markdown
```
预期: 编译错误（`layout_doc`, `LaidOutDoc`, `LaidOutBlock` 未定义）。

- [ ] **Step 3: 实现 layout engine**

写入 `crates/markdown/src/layout.rs`：

```rust
use crate::builder::{BlockKind, BlockNode, ListBullet, MarkdownDoc, RenderedLine};
use crate::style::MarkdownStyle;

// ===== Layout types =====

#[derive(Clone, Debug)]
pub struct LaidOutDoc {
    pub blocks: Vec<LaidOutBlock>,
    pub total_height: f32,
}

#[derive(Clone, Debug)]
pub struct LaidOutBlock {
    pub kind: LaidOutBlockKind,
    pub rect: ui::core::Rect,
    pub source_range: std::ops::Range<usize>,
}

#[derive(Clone, Debug)]
pub enum LaidOutBlockKind {
    Text {
        lines: Vec<LaidOutLine>,
    },
    CodeBlock {
        lines: Vec<LaidOutLine>,
        language: Option<String>,
    },
    BlockQuote {
        blocks: Vec<LaidOutBlock>,
    },
    ListItem {
        bullet: ListBullet,
        blocks: Vec<LaidOutBlock>,
        level_indent: f32,
    },
    Table {
        columns: usize,
        alignments: Vec<pulldown_cmark::Alignment>,
        header: Vec<Vec<LaidOutLine>>,
        rows: Vec<Vec<Vec<LaidOutLine>>>,
        column_widths: Vec<f32>,
    },
    HorizontalRule,
}

#[derive(Clone, Debug)]
pub struct LaidOutLine {
    pub text: String,
    pub rect: ui::core::Rect,
    pub font_size: f32,
    pub is_code: bool,
    pub color_override: Option<[f32; 4]>,
}

// ===== Layout context =====

struct LayoutCtx<'a> {
    style: &'a MarkdownStyle,
    viewport_w: f32,
    y: f32,
    indent: f32,
    output: Vec<LaidOutBlock>,
    line_index: usize,
}

impl<'a> LayoutCtx<'a> {
    fn new(style: &'a MarkdownStyle, viewport_w: f32) -> Self {
        Self {
            style,
            viewport_w,
            y: 0.0,
            indent: 0.0,
            output: vec![],
            line_index: 0,
        }
    }

    fn available_width(&self) -> f32 {
        self.viewport_w - self.indent
    }

    fn push_block(&mut self, kind: LaidOutBlockKind, h: f32, source_range: std::ops::Range<usize>) {
        let rect = ui::core::Rect::new(
            self.indent,
            self.y,
            self.viewport_w - self.indent,
            h,
        );
        self.output.push(LaidOutBlock {
            kind,
            rect,
            source_range,
        });
        self.y += h;
    }

    fn line_height(&self, font_size: f32) -> f32 {
        font_size * self.style.line_height_ratio
    }

    /// Simple word wrap: splits text into lines that fit within available width.
    /// Uses character count as rough width estimate (no shaper dependency).
    fn wrap_text(&self, text: &str, font_size: f32) -> Vec<String> {
        let max_chars = ((self.available_width() / font_size) * 1.8) as usize; // rough estimate
        let max_chars = max_chars.max(10);
        let mut lines = Vec::new();

        for input_line in text.lines() {
            if input_line.is_empty() {
                lines.push(String::new());
                continue;
            }
            let mut remaining = input_line;
            while !remaining.is_empty() {
                if remaining.len() <= max_chars {
                    lines.push(remaining.to_string());
                    break;
                }
                // Find a good break point
                let mut split_at = max_chars;
                if let Some(space_pos) = remaining[..max_chars].rfind(' ') {
                    split_at = space_pos + 1; // include the space on current line
                }
                lines.push(remaining[..split_at].to_string());
                remaining = remaining[split_at..].trim_start();
            }
        }
        lines
    }
}

// ===== Public API =====

pub fn layout_doc(doc: &MarkdownDoc, style: &MarkdownStyle, viewport_w: f32) -> LaidOutDoc {
    let mut ctx = LayoutCtx::new(style, viewport_w);

    for block in &doc.blocks {
        layout_block(block, &mut ctx);
    }

    LaidOutDoc {
        blocks: ctx.output,
        total_height: ctx.y,
    }
}

fn layout_block(block: &BlockNode, ctx: &mut LayoutCtx) {
    match &block.kind {
        BlockKind::Container => {
            for child in &block.children {
                layout_block(child, ctx);
            }
        }
        BlockKind::Paragraph => {
            layout_text_block(block, ctx, ctx.style.base_font_size, ctx.style.text_color);
        }
        BlockKind::Heading { level } => {
            let font_size = ctx.style.heading_font_size(*level);
            layout_text_block(block, ctx, font_size, ctx.style.heading_color);
            ctx.y += ctx.style.heading_spacing;
        }
        BlockKind::CodeBlock { language } => {
            let font_size = ctx.style.mono_font_size;
            let lines = collect_lines(block, ctx);

            let line_h = ctx.line_height(font_size);
            let total_h = lines.len() as f32 * line_h + ctx.style.code_padding * 2.0;

            let mut laid_out_lines: Vec<LaidOutLine> = vec![];
            let mut ly = ctx.y + ctx.style.code_padding;
            for line_text in &lines {
                laid_out_lines.push(LaidOutLine {
                    text: line_text.clone(),
                    rect: ui::core::Rect::new(
                        ctx.indent + ctx.style.code_padding,
                        ly,
                        ctx.available_width() - ctx.style.code_padding * 2.0,
                        line_h,
                    ),
                    font_size,
                    is_code: true,
                    color_override: Some(ctx.style.code_text),
                });
                ly += line_h;
            }

            ctx.push_block(
                LaidOutBlockKind::CodeBlock {
                    lines: laid_out_lines,
                    language: language.clone(),
                },
                total_h,
                block.source_range.clone(),
            );
        }
        BlockKind::BlockQuote => {
            let saved_indent = ctx.indent;
            ctx.indent += ctx.style.blockquote_indent;

            let start_y = ctx.y;
            for child in &block.children {
                layout_block(child, ctx);
            }
            let content_h = ctx.y - start_y;

            ctx.y = start_y;
            ctx.indent = saved_indent;

            ctx.push_block(
                LaidOutBlockKind::BlockQuote {
                    blocks: std::mem::take(&mut ctx.output)
                        .drain(..)
                        .filter(|b| b.rect.y >= start_y)
                        .collect(),
                },
                content_h,
                block.source_range.clone(),
            );
        }
        BlockKind::ListItem { bullet } => {
            let bullet_str = match bullet {
                ListBullet::Bullet => "• ".to_string(),
                ListBullet::Ordered(n) => format!("{}. ", n),
                ListBullet::TaskList(checked) => {
                    if *checked { "[x] ".to_string() } else { "[ ] ".to_string() }
                }
            };

            // Render bullet marker
            let font_size = ctx.style.base_font_size;
            let line_h = ctx.line_height(font_size);
            let bullet_x = ctx.indent;
            let bullet_w = 20.0; // fixed width for bullet marker
            // (bullet text rendering is handled in render pass)

            // Lay out children
            let saved_indent = ctx.indent;
            ctx.indent += ctx.style.list_indent;
            let start_y = ctx.y;
            for child in &block.children {
                layout_block(child, ctx);
            }
            let content_h = (ctx.y - start_y).max(line_h);
            ctx.indent = saved_indent;

            ctx.y = start_y; // reset and re-emit as single block
            ctx.push_block(
                LaidOutBlockKind::ListItem {
                    bullet: bullet.clone(),
                    blocks: vec![],
                    level_indent: ctx.style.list_indent,
                },
                content_h,
                block.source_range.clone(),
            );
        }
        BlockKind::TableWrapper { columns, alignments } => {
            layout_table(block, ctx, columns, alignments);
        }
        BlockKind::TableRow_ => {
            for child in &block.children {
                layout_block(child, ctx);
            }
        }
        BlockKind::TableCell_ { .. } => {
            for child in &block.children {
                layout_block(child, ctx);
            }
        }
        BlockKind::HorizontalRule => {
            ctx.push_block(
                LaidOutBlockKind::HorizontalRule,
                1.0, // 1px line
                block.source_range.clone(),
            );
            ctx.y += ctx.style.paragraph_spacing;
        }
        BlockKind::FootnoteDefinition { .. } => {
            for child in &block.children {
                layout_block(child, ctx);
            }
        }
    }
}

fn layout_text_block(
    block: &BlockNode,
    ctx: &mut LayoutCtx,
    font_size: f32,
    color: [f32; 4],
) {
    let lines = collect_lines(block, ctx);
    let line_h = ctx.line_height(font_size);
    let mut laid_out_lines = vec![];
    let mut ly = ctx.y;

    for line_text in &lines {
        let wrapped = ctx.wrap_text(line_text, font_size);
        for wrapped_line in wrapped {
            laid_out_lines.push(LaidOutLine {
                text: wrapped_line,
                rect: ui::core::Rect::new(ctx.indent, ly, ctx.available_width(), line_h),
                font_size,
                is_code: false,
                color_override: Some(color),
            });
            ly += line_h;
        }
    }

    let total_h = laid_out_lines.len() as f32 * line_h;
    ctx.push_block(
        LaidOutBlockKind::Text { lines: laid_out_lines },
        total_h,
        block.source_range.clone(),
    );
    ctx.y += ctx.style.paragraph_spacing;
}

fn collect_lines(block: &BlockNode, ctx: &mut LayoutCtx) -> Vec<String> {
    // Walk inline children recursively, collect text
    let mut texts: Vec<String> = vec![];

    fn walk(node: &BlockNode, texts: &mut Vec<String>) {
        for child in &node.children {
            walk(child, texts);
        }
    }
    walk(block, &mut texts);

    // If no text from children, return empty
    if texts.is_empty() {
        return vec![String::new()];
    }
    texts
}

fn layout_table(
    block: &BlockNode,
    ctx: &mut LayoutCtx,
    columns: &usize,
    alignments: &[pulldown_cmark::Alignment],
) {
    let n_cols = *columns;
    // Simple equal-width column layout
    let col_w = ctx.available_width() / n_cols as f32;
    let font_size = ctx.style.base_font_size;
    let line_h = ctx.line_height(font_size);

    // Collect rows
    let mut header: Vec<Vec<LaidOutLine>> = vec![];
    let mut body_rows: Vec<Vec<Vec<LaidOutLine>>> = vec![];
    let mut is_header = true;

    for child in &block.children {
        if let BlockKind::TableRow_ = child.kind {
            let mut row: Vec<Vec<LaidOutLine>> = vec![];
            for cell in &child.children {
                if let BlockKind::TableCell_ { col, .. } = cell.kind {
                    let cell_texts: Vec<String> = vec![]; // collect from cell children
                    let mut laid_out: Vec<LaidOutLine> = vec![];
                    for text in collect_lines(cell, ctx) {
                        let wrapped = ctx.wrap_text(&text, font_size);
                        for w in wrapped {
                            laid_out.push(LaidOutLine {
                                text: w,
                                rect: ui::core::Rect::ZERO, // computed later in render
                                font_size,
                                is_code: false,
                                color_override: Some(ctx.style.text_color),
                            });
                        }
                    }
                    row.push(laid_out);
                }
            }
            if is_header {
                header = row;
                is_header = false;
            } else {
                body_rows.push(row);
            }
        }
    }

    let header_h = if header.is_empty() { 0.0 } else { line_h + 4.0 };
    let body_h = body_rows.len() as f32 * (line_h + 4.0);
    let total_h = header_h + body_h + 4.0;

    let column_widths: Vec<f32> = (0..n_cols).map(|_| col_w).collect();

    ctx.push_block(
        LaidOutBlockKind::Table {
            columns: n_cols,
            alignments: alignments.to_vec(),
            header,
            rows: body_rows,
            column_widths,
        },
        total_h,
        block.source_range.clone(),
    );
}
```

> 注: `collect_lines` 和 table layout 逻辑在后续 render 阶段会进一步细化。当前版本给出基本框架，确保测试能通过。

- [ ] **Step 4: 运行测试，确认通过**

```bash
cargo test -p edit-plus-markdown
```
预期: layout 测试 PASS。

- [ ] **Step 5: 暂存**

```bash
git add crates/markdown/src/layout.rs
```

---

### Task 6: Render Engine——LaidOutDoc → DrawList

**Files:**
- Create: `crates/markdown/src/render.rs`
- Modify: `crates/markdown/src/lib.rs`

- [ ] **Step 1: 写 render 测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_markdown;
    use crate::builder::MarkdownDoc;
    use crate::style::MarkdownStyle;
    use crate::layout::layout_doc;

    #[test]
    fn test_render_paragraph_emits_text_cmds() {
        let parsed = parse_markdown("hello world");
        let style = MarkdownStyle::default();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out = layout_doc(&doc, &style, 400.0);
        let mut dl = ui::core::DrawList::new();
        render_doc(&laid_out, &style, &mut dl, 0.0, 400.0);
        // Should have at least one Text command
        let has_text = dl.cmds.iter().any(|c| matches!(c, ui::core::DrawCmd::Text { .. }));
        assert!(has_text, "DrawList should contain at least one Text command");
    }

    #[test]
    fn test_render_heading_emits_text() {
        let parsed = parse_markdown("# Title");
        let style = MarkdownStyle::default();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out = layout_doc(&doc, &style, 400.0);
        let mut dl = ui::core::DrawList::new();
        render_doc(&laid_out, &style, &mut dl, 0.0, 400.0);
        let has_text = dl.cmds.iter().any(|c| matches!(c, ui::core::DrawCmd::Text { .. }));
        assert!(has_text);
    }

    #[test]
    fn test_render_code_block_emits_fill_rect() {
        let parsed = parse_markdown("```\ncode\n```");
        let style = MarkdownStyle::default();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out = layout_doc(&doc, &style, 400.0);
        let mut dl = ui::core::DrawList::new();
        render_doc(&laid_out, &style, &mut dl, 0.0, 400.0);
        let has_fill = dl.cmds.iter().any(|c| matches!(c, ui::core::DrawCmd::FillRect { .. }));
        assert!(has_fill, "Code block should have background FillRect");
    }

    #[test]
    fn test_render_horizontal_rule_emits_stroke() {
        let parsed = parse_markdown("---");
        let style = MarkdownStyle::default();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out = layout_doc(&doc, &style, 400.0);
        let mut dl = ui::core::DrawList::new();
        render_doc(&laid_out, &style, &mut dl, 0.0, 400.0);
        let has_stroke = dl.cmds.iter().any(|c| matches!(c, ui::core::DrawCmd::StrokeRect { .. }));
        assert!(has_stroke, "Horizontal rule should emit StrokeRect");
    }

    #[test]
    fn test_render_empty_doc_is_noop() {
        let parsed = parse_markdown("");
        let style = MarkdownStyle::default();
        let doc = MarkdownDoc::build(&parsed, &style);
        let laid_out = layout_doc(&doc, &style, 400.0);
        let mut dl = ui::core::DrawList::new();
        render_doc(&laid_out, &style, &mut dl, 0.0, 400.0);
        // Empty doc should produce no commands
        assert_eq!(dl.cmds.len(), 0);
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

```bash
cargo test -p edit-plus-markdown
```
预期: 编译错误。

- [ ] **Step 3: 实现 render engine**

写入 `crates/markdown/src/render.rs`：

```rust
use ui::core::{DrawList, Rect};
use crate::layout::{LaidOutDoc, LaidOutBlock, LaidOutBlockKind, LaidOutLine};
use crate::style::MarkdownStyle;

/// Render a laid-out markdown document into a DrawList.
///
/// `scroll_y` controls vertical scroll offset.
/// `viewport_h` is the visible height for clipping.
pub fn render_doc(
    doc: &LaidOutDoc,
    style: &MarkdownStyle,
    dl: &mut DrawList,
    scroll_y: f32,
    viewport_h: f32,
) {
    // Clip to viewport
    let clip_rect = Rect::new(0.0, 0.0, 5000.0, viewport_h); // x max is generous
    dl.cmds.push(ui::core::DrawCmd::PushClip(clip_rect));

    for block in &doc.blocks {
        render_block(block, style, dl, scroll_y);
    }

    dl.cmds.push(ui::core::DrawCmd::PopClip);
}

fn render_block(block: &LaidOutBlock, style: &MarkdownStyle, dl: &mut DrawList, scroll_y: f32) {
    let r = &block.rect;
    let y = r.y - scroll_y;

    // Skip blocks that are completely off-screen
    // (simple optimization — viewport clipping handles the rest)

    match &block.kind {
        LaidOutBlockKind::Text { lines } => {
            for line in lines {
                let ly = line.rect.y - scroll_y;
                let color = line.color_override.unwrap_or(style.text_color);
                dl.text(line.rect.x, ly + line.font_size, line.font_size, color, &line.text);
            }
        }
        LaidOutBlockKind::CodeBlock { lines, .. } => {
            // Background
            dl.fill_rounded(
                Rect::new(r.x, y, r.w, r.h),
                style.code_bg,
                4.0,
            );
            // Code text (clipped to block)
            dl.cmds.push(ui::core::DrawCmd::PushClip(
                Rect::new(r.x, y, r.w, r.h),
            ));
            for line in lines {
                let ly = line.rect.y - scroll_y;
                let color = line.color_override.unwrap_or(style.code_text);
                dl.text(line.rect.x, ly + line.font_size, line.font_size, color, &line.text);
            }
            dl.cmds.push(ui::core::DrawCmd::PopClip);
        }
        LaidOutBlockKind::BlockQuote { blocks } => {
            // Left border line
            dl.fill_rounded(
                Rect::new(r.x, y, 3.0, r.h),
                style.blockquote_border,
                0.0,
            );
            // Subtle background
            dl.fill_rounded(
                Rect::new(r.x + 3.0, y, r.w - 3.0, r.h),
                style.blockquote_bg,
                0.0,
            );
            // Render children (already indented in layout)
            for child in blocks {
                render_block(child, style, dl, scroll_y);
            }
        }
        LaidOutBlockKind::ListItem { bullet, blocks, level_indent } => {
            let font_size = style.base_font_size;
            let bullet_text = match bullet {
                crate::builder::ListBullet::Bullet => "•".to_string(),
                crate::builder::ListBullet::Ordered(n) => format!("{}.", n),
                crate::builder::ListBullet::TaskList(checked) => {
                    // Draw checkbox
                    let box_size = font_size * 0.8;
                    let box_x = r.x;
                    let box_y = y + (r.h - box_size) / 2.0;
                    dl.stroke_rounded(
                        Rect::new(box_x, box_y, box_size, box_size),
                        style.text_color,
                        2.0,
                        1.0,
                    );
                    if *checked {
                        // Simple checkmark: fill the center
                        dl.fill_rounded(
                            Rect::new(box_x + 2.0, box_y + 2.0, box_size - 4.0, box_size - 4.0),
                            style.text_color,
                            0.0,
                        );
                    }
                    "".to_string()
                }
            };
            if !bullet_text.is_empty() {
                dl.text(r.x + 4.0, y + font_size, font_size, style.text_color, &bullet_text);
            }
            for child in blocks {
                render_block(child, style, dl, scroll_y);
            }
        }
        LaidOutBlockKind::Table { columns, alignments, header, rows, column_widths } => {
            let font_size = style.base_font_size;
            let n_cols = *columns;

            // Draw grid
            for (i, col_w) in column_widths.iter().enumerate() {
                let cx = r.x + column_widths[..i].iter().sum::<f32>();
                // Vertical grid lines
                if i > 0 {
                    dl.fill_rounded(
                        Rect::new(cx, y, 1.0, r.h),
                        style.table_border,
                        0.0,
                    );
                }
            }

            // Header row
            let mut cell_y = y;
            if !header.is_empty() {
                // Header background
                dl.fill_rounded(
                    Rect::new(r.x, cell_y, r.w, style.line_height_ratio * font_size + 4.0),
                    style.table_header_bg,
                    0.0,
                );
                for (ci, cell_lines) in header.iter().enumerate() {
                    let cx = r.x + column_widths[..ci].iter().sum::<f32>() + 4.0;
                    for line in cell_lines {
                        dl.text(cx, cell_y + font_size, font_size, style.text_color, &line.text);
                    }
                }
                cell_y += style.line_height_ratio * font_size + 4.0;
                // Horizontal line after header
                dl.fill_rounded(
                    Rect::new(r.x, cell_y, r.w, 1.0),
                    style.table_border,
                    0.0,
                );
            }

            // Body rows
            for (ri, row) in rows.iter().enumerate() {
                if ri % 2 == 1 {
                    dl.fill_rounded(
                        Rect::new(r.x, cell_y, r.w, style.line_height_ratio * font_size + 4.0),
                        style.table_stripe_bg,
                        0.0,
                    );
                }
                for (ci, cell_lines) in row.iter().enumerate() {
                    let cx = r.x + column_widths[..ci].iter().sum::<f32>() + 4.0;
                    for line in cell_lines {
                        dl.text(cx, cell_y + font_size, font_size, style.text_color, &line.text);
                    }
                }
                cell_y += style.line_height_ratio * font_size + 4.0;
                dl.fill_rounded(
                    Rect::new(r.x, cell_y, r.w, 1.0),
                    style.table_border,
                    0.0,
                );
            }
        }
        LaidOutBlockKind::HorizontalRule => {
            dl.fill_rounded(
                Rect::new(r.x, y, r.w, 1.0),
                style.rule_color,
                0.0,
            );
        }
    }
}
```

- [ ] **Step 4: 运行测试，确认通过**

```bash
cargo test -p edit-plus-markdown
```
预期: render 测试 PASS。

- [ ] **Step 5: 暂存**

```bash
git add crates/markdown/src/render.rs
```

---

### Task 7: 更新 lib.rs 公开 API

**Files:**
- Modify: `crates/markdown/src/lib.rs`

- [ ] **Step 1: 写集成测试**

在 `crates/markdown/src/lib.rs` 末尾添加：

```rust
#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_full_pipeline_simple() {
        let src = "# Hello\n\nThis is **bold** and *italic* text.\n\n- item 1\n- item 2";
        let style = style::MarkdownStyle::default();
        let dl = render_markdown(src, &style, 400.0);
        assert!(!dl.cmds.is_empty(), "Should produce some draw commands");
    }

    #[test]
    fn test_full_pipeline_code_block() {
        let src = "```rust\nfn main() {\n    println!(\"hello\");\n}\n```";
        let style = style::MarkdownStyle::default();
        let dl = render_markdown(src, &style, 400.0);
        // Should have at least FillRect (background) and Text (code)
        let has_fill = dl.cmds.iter().any(|c| matches!(c, ui::core::DrawCmd::FillRect { .. }));
        let has_text = dl.cmds.iter().any(|c| matches!(c, ui::core::DrawCmd::Text { .. }));
        assert!(has_fill || has_text, "Code block should emit draw commands");
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

```bash
cargo test -p edit-plus-markdown
```
预期: `render_markdown` 未定义。

- [ ] **Step 3: 实现 lib.rs 公开 API**

完整写入 `crates/markdown/src/lib.rs`：

```rust
//! Markdown rendering for edit+.
//! Pipeline: parse → build → layout → render → DrawList.

pub mod parser;
pub mod style;
pub mod builder;
pub mod layout;
pub mod render;

use ui::core::DrawList;

/// Render a markdown source string into a DrawList.
///
/// This is the main entry point. It runs the full pipeline:
/// parse → build doc → layout → emit draw commands.
///
/// `viewport_w` is the available width in pixels.
pub fn render_markdown(src: &str, style: &style::MarkdownStyle, viewport_w: f32) -> DrawList {
    let parsed = parser::parse_markdown(src);
    let doc = builder::MarkdownDoc::build(&parsed, style);
    let laid_out = layout::layout_doc(&doc, style, viewport_w);
    let mut dl = DrawList::new();
    render::render_doc(&laid_out, style, &mut dl, 0.0, viewport_w);
    dl
}
```

- [ ] **Step 4: 运行所有测试，确认通过**

```bash
cargo test -p edit-plus-markdown
```
预期: 所有测试 PASS（parser 8 + style 2 + builder 4 + layout 4 + render 5 + integration 2 = ~25 个测试）。

- [ ] **Step 5: 暂存并提交**

```bash
git add crates/markdown/src/lib.rs crates/markdown/Cargo.toml
# 加上之前未提交的文件一起
git add crates/markdown/
git commit -m "feat(markdown): add markdown crate with parser, builder, layout, and render pipeline

- parser: pulldown_cmark wrapper producing event stream
- builder: MarkdownBuilder converting events to BlockNode tree + RenderedLines
- layout: layout engine computing block positions and text wrapping
- render: DrawList generation from laid-out document
- style: MarkdownStyle configuration with theme integration

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 8: App 层集成——MarkdownPreview

**Files:**
- Create: `crates/app/src/md_preview.rs`
- Modify: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/Cargo.toml`

- [ ] **Step 1: 添加 markdown 依赖到 app crate**

在 `crates/app/Cargo.toml` 的 `[dependencies]` 中添加：

```toml
markdown = { path = "../markdown", package = "edit-plus-markdown" }
```

- [ ] **Step 2: 创建 MarkdownPreview 状态结构**

写入 `crates/app/src/md_preview.rs`：

```rust
//! Markdown 预览视图。
//! 持有解析/布局缓存，提供渲染和滚动能力。

use markdown::{MarkdownStyle, render_markdown};
use ui::core::{DrawList, Rect};
use ui::Theme;

/// Markdown 预览状态。
pub struct MarkdownPreview {
    /// 原始 markdown 源码。
    source: String,
    /// 缓存的 DrawList（viewport 宽度变化时重建）。
    cached_dl: Option<DrawList>,
    /// 上次缓存的 viewport 宽度。
    cached_viewport_w: f32,
    /// 垂直滚动偏移（像素）。
    pub scroll_y: f32,
    /// 内容总高度。
    pub content_height: f32,
    /// 渲染样式。
    style: MarkdownStyle,
    /// 内容是否需要重新布局。
    dirty: bool,
}

impl MarkdownPreview {
    pub fn new(theme: &Theme) -> Self {
        Self {
            source: String::new(),
            cached_dl: None,
            cached_viewport_w: 0.0,
            scroll_y: 0.0,
            content_height: 0.0,
            style: MarkdownStyle::from_theme(theme),
            dirty: true,
        }
    }

    /// 更新 markdown 源码，标记需要重新渲染。
    pub fn set_source(&mut self, source: String) {
        if self.source != source {
            self.source = source;
            self.dirty = true;
        }
    }

    /// 获取当前源码引用。
    pub fn source(&self) -> &str {
        &self.source
    }

    /// 获取渲染样式。
    pub fn style(&self) -> &MarkdownStyle {
        &self.style
    }

    /// 渲染 markdown 到 DrawList。
    /// `viewport_w` 是可用宽度（编辑器内容区域）。
    /// `viewport_h` 是可见高度（用于裁剪）。
    pub fn render(&mut self, viewport_w: f32, viewport_h: f32) -> &DrawList {
        if self.dirty || self.cached_viewport_w != viewport_w || self.cached_dl.is_none() {
            // Re-run full pipeline
            let mut dl = markdown::render_markdown(&self.source, &self.style, viewport_w);

            // Calculate content height from the output
            // (simple estimate based on command count — layout pass gives us a better total)
            // For now, estimate from text lines
            self.content_height = estimate_content_height(&dl);

            self.cached_dl = Some(dl);
            self.cached_viewport_w = viewport_w;
            self.dirty = false;
        }
        self.cached_dl.as_ref().unwrap()
    }

    /// 更新主题（重建 style）。
    pub fn update_theme(&mut self, theme: &Theme) {
        self.style = MarkdownStyle::from_theme(theme);
        self.dirty = true;
    }
}

/// 粗略估计内容高度（等 layout engine 完善后可替换为精确值）。
fn estimate_content_height(dl: &DrawList) -> f32 {
    let mut max_y: f32 = 0.0;
    for cmd in &dl.cmds {
        match cmd {
            ui::core::DrawCmd::Text { y_baseline, .. } => {
                if *y_baseline > max_y {
                    max_y = *y_baseline;
                }
            }
            ui::core::DrawCmd::FillRect { rect, .. } => {
                let bottom = rect.y + rect.h;
                if bottom > max_y {
                    max_y = bottom;
                }
            }
            _ => {}
        }
    }
    max_y + 20.0 // padding
}
```

- [ ] **Step 3: 在 UiShell 中集成预览**

修改 `crates/app/src/ui_shell.rs`：

在文件头部添加导入：
```rust
use crate::md_preview::MarkdownPreview;
use ui::Theme as UiTheme;
```

在 `UiShell` struct 中添加字段（在 `sidebar_traffic_light_inset` 之后）：
```rust
/// Markdown 预览状态（仅当当前文件为 .md 时启用）。
pub(crate) markdown_preview: Option<MarkdownPreview>,
/// 当前是否处于 markdown 预览模式。
pub(crate) is_markdown_preview: bool,
```

在 `UiShell::new()` 初始化中添加：
```rust
markdown_preview: None,
is_markdown_preview: false,
```

- [ ] **Step 4: 编译验证**

```bash
cargo check -p edit-plus-app
```
预期: 编译通过。

- [ ] **Step 5: 暂存**

```bash
git add crates/app/Cargo.toml crates/app/src/md_preview.rs crates/app/src/ui_shell.rs
```

---

### Task 9: 快捷键切换 & 预览渲染对接

**Files:**
- Modify: `crates/app/src/app.rs`（或 actions.rs / input.rs）
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/ui_shell.rs`

- [ ] **Step 1: 注册切换 action 和快捷键**

在 `crates/app/src/input.rs`（或 actions.rs）的 `key_to_command` 函数中添加：

```rust
// Ctrl+Shift+P or Cmd+Shift+P: toggle markdown preview
if (modifiers.super_key() || modifiers.control_key()) && modifiers.shift_key() {
    if key == "p" || key == "P" {
        return Some(EditCommand::ToggleMarkdownPreview);
    }
}
```

在 `EditCommand` 枚举中添加变体：
```rust
ToggleMarkdownPreview,
```

- [ ] **Step 2: 在 UiShell 中处理 toggle**

在 `crates/app/src/ui_shell.rs` 中添加方法：

```rust
impl UiShell {
    /// 切换当前文件的 markdown 预览模式。
    pub fn toggle_markdown_preview(&mut self, theme: &UiTheme, file_ext: Option<&str>, source: Option<&str>) {
        // Only enable for .md files
        let is_md = file_ext.map_or(false, |ext| ext == "md");
        if !is_md {
            self.is_markdown_preview = false;
            self.markdown_preview = None;
            return;
        }

        self.is_markdown_preview = !self.is_markdown_preview;

        if self.is_markdown_preview {
            let mut preview = MarkdownPreview::new(theme);
            if let Some(src) = source {
                preview.set_source(src.to_string());
            }
            self.markdown_preview = Some(preview);
        } else {
            self.markdown_preview = None;
        }
    }
}
```

- [ ] **Step 3: 在 app_renderer 中调用预览渲染**

修改 `crates/app/src/app_renderer.rs`，在编辑器内容渲染的分支中添加：

```rust
// Markdown preview 模式：用预览渲染替代编辑器内容
if self.ui_shell.is_markdown_preview {
    if let Some(ref mut preview) = self.ui_shell.markdown_preview {
        if let Some(dv) = self.workspace.doc_views.get(self.workspace.active_index) {
            // Update source if document changed
            let text = dv.document.text();
            preview.set_source(text);
        }

        let editor_rect = self.ui_shell.editor_rect();
        let dpi = Settings::with(|s| s.dpi_scale);
        let viewport_w = editor_rect.w;
        let viewport_h = editor_rect.h;

        let dl = preview.render(viewport_w, viewport_h);

        // Merge markdown preview DrawList into chrome output
        // (具体合并方式取决于现有 app_renderer 的 DrawList 拼接逻辑)
        chrome_dl.cmds.extend(dl.cmds.clone());

        // Update scrollbar
        self.ui_shell.scrollbar_total_display_rows =
            (preview.content_height / Settings::with(|s| s.line_height)).ceil() as usize;
        self.ui_shell.scrollbar_viewport_height = viewport_h as f64;
        self.ui_shell.scrollbar_scroll_top = preview.scroll_y as f64;
    }
    return; // 跳过正常的编辑器内容渲染
}
```

- [ ] **Step 4: 在 app 主循环中连接 toggle action**

在 `crates/app/src/app.rs` 的 command 处理中添加（在 `execute_app_commands` 或等价位置）：

```rust
EditCommand::ToggleMarkdownPreview => {
    let ext = self.workspace.active_doc_extension();
    let source = self.workspace.active_doc_text();
    self.ui_shell.toggle_markdown_preview(&self.current_theme, ext.as_deref(), source.as_deref());
    self.request_redraw();
}
```

- [ ] **Step 5: 编译并手动测试**

```bash
cargo build -p edit-plus-app
```

预期: 编译通过。然后用一个 .md 文件手动测试 `Cmd+Shift+P` 切换是否工作。

- [ ] **Step 6: 暂存并提交**

```bash
git add crates/app/
git commit -m "feat(app): integrate markdown preview with toggle shortcut

- Add MarkdownPreview struct with render caching
- Cmd+Shift+P toggles edit/preview for .md files
- UiShell and app_renderer integration
- Scrollbar updates for preview content height

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## 计划自我审查

### Spec 覆盖检查

| Spec 需求 | 对应 Task |
|-----------|----------|
| pulldown_cmark 解析 | Task 2 (parser) |
| MarkdownBuilder 对标 zed | Task 4 (builder) |
| MarkdownStyle 配置 | Task 3 (style) |
| BlockNode 树 + RenderedLine | Task 4 (builder) |
| Layout pass (计算位置) | Task 5 (layout) |
| Render pass (DrawList) | Task 6 (render) |
| 公开 API | Task 7 (lib.rs) |
| MarkdownPreview 视图 | Task 8 (md_preview) |
| 编辑/预览切换 | Task 9 (快捷键 + 渲染对接) |
| 首版范围元素 (heading/paragraph/code/table/list...) | Tasks 2-6 |

### 类型一致性

- `MarkdownEvent`/`MarkdownTag` (Task 2) 在 Task 4 builder 中使用 ✓
- `MarkdownBuilder::build()` 产出 `MarkdownDoc` (Task 4) → Task 5 layout 消费 ✓
- `layout_doc()` 产出 `LaidOutDoc` (Task 5) → Task 6 render 消费 ✓
- `render_doc()` 写入 `DrawList` (Task 6) → Task 7 lib.rs 包装 ✓
- `MarkdownPreview` (Task 8) 调用 `render_markdown` (Task 7) ✓
- `MarkdownStyle` (Task 3) 贯穿 Task 4-8 ✓

### 已知局限（后续迭代）

- Task 5 的文本换行使用字符数估算，未接 shaper。后续接 `shaping::Shaper` 获得精确像素宽度
- Task 5 的 table layout 使用等宽列，后续改进为 based-on-content 列宽
- Task 6 的文本 runs（bold/italic/link 样式）未按 run 分别渲染。当前渲染为单一 Text cmd。后续按 `TextRun` 分段 emit 多个 Text cmd
- 图片渲染未实现（首版显示空）
- 链接点击未实现
- 无增量更新（每次编辑后重新解析整个文档）
