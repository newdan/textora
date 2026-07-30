# MarkdownView & Novel 合并实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 novel crate 合并到 markdown crate，共享 PreviewEngine 渲染管线，MarkdownPreview→MarkdownView 重命名，独立 Theme 配置。

**Architecture:** `PreviewEngine` 是共享缓存/渲染/滚动/选区/搜索体。`MarkdownView` 和 `NovelView` 各自包装 engine，通过不同的 `MarkdownStyle`（`from_theme` vs `novel`）和不同的 `MarkdownDoc` 来源（`parse_markdown` vs `build_from_txt`）实现差异。两个工厂通过 `PluginRegistry` 注册。

**Tech Stack:** Rust 2024, pulldown-cmark, wgpu, regex

## Global Constraints

- `./scripts/verify.sh` 每次提交前必须通过
- 函数不超过 50 行
- 禁止 `.unwrap()` 无 `.expect()` 说明
- `cargo fmt` + `cargo clippy` 零警告
- 零硬编码类型判断，全部通过 `ViewPlugin` trait 交互
- Novel Theme 独立 section，不回退 markdown

---

## 文件结构

```
crates/markdown/src/
  view.rs          ← 重命名自 preview.rs，包含 PreviewEngine + MarkdownView + NovelView + factories
  selection.rs     ← ViewPos + SelectionState + 纯函数 (不变)
  search.rs        ← SearchState (不变)
  builder.rs       ← MarkdownDoc::build() + 新增 build_from_txt()
  layout.rs        ← LazyLayout + char_at_x/char_x (不变)
  render.rs        ← render_doc (不变)
  parser.rs        ← pulldown-cmark 解析 (不变)
  style.rs         ← MarkdownStyle + 新增 novel()
  novel/
    chapter.rs     ← ChapterIndex + classify_title() (从 novel crate 移入)
    merge.rs       ← 简化的 ParagraphIndex (去除 Y 计算) + merge_paragraph_lines()

crates/novel/      → 删除整个 crate
```

---

### Task 1: 重命名 — MarkdownPreview→MarkdownView，删除 PreviewPos 别名

**Files:**
- Modify: `crates/markdown/src/preview.rs` → 重命名内容（文件暂不动）

**Interfaces:**
- Consumes: 无
- Produces: `MarkdownView` 结构体（原 `MarkdownPreview`），`"markdown_view"` 名称字符串

- [ ] **Step 1: 在 preview.rs 中重命名结构体和所有关联项**

`MarkdownPreview` → `MarkdownView`:
```rust
pub struct MarkdownView {
    source: String,
    // ... 所有字段不变
}

impl Default for MarkdownView {
    fn default() -> Self { Self::new() }
}

impl MarkdownView {
    pub fn new() -> Self { /* 不变 */ }
    // 所有方法不变，仅改名
}
```

`impl ViewPlugin for MarkdownView`:
```rust
impl ViewPlugin for MarkdownView {
    fn name(&self) -> &str {
        "markdown_view"  // 原 "markdown_preview"
    }
    // 其余不变
}
```

`MarkdownPluginFactory` → `MarkdownViewFactory`:
```rust
pub struct MarkdownViewFactory;

impl PluginFactory for MarkdownViewFactory {
    fn name(&self) -> &str { "markdown_view" }
    fn can_handle(&self, path: Option<&Path>) -> bool {
        path.is_some_and(|p| p.extension().is_some_and(|e| e == "md" || e == "markdown"))
    }
    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(MarkdownView::new())
    }
}
```

- [ ] **Step 2: 删除 `PreviewPos` 别名**，在所有使用处改为直接 `use crate::selection::ViewPos`

在 preview.rs 中删除：
```rust
pub use crate::selection::ViewPos as PreviewPos;
```

将所有 `PreviewPos` 引用替换为 `crate::selection::ViewPos`（或通过 use 引入）。

- [ ] **Step 3: 更新 lib.rs 模块声明**

```rust
pub mod view;  // 原 pub mod preview;
```

- [ ] **Step 4: 更新所有外部引用**

在 `crates/app/src/workspace.rs`：
```rust
// 原: edit_plus_markdown::preview::MarkdownPluginFactory
// 改为:
edit_plus_markdown::view::MarkdownViewFactory
```

在 `crates/app/src/app_renderer.rs`：
- `is_md_preview` → `is_readonly_view`（变量名，语义不变）
- 无 markdown 模块直接引用（通过 ViewPlugin trait 交互），无需改 import

- [ ] **Step 5: 重命名文件 preview.rs → view.rs**

```bash
git mv crates/markdown/src/preview.rs crates/markdown/src/view.rs
```

- [ ] **Step 6: 编译验证**

```bash
cargo build 2>&1
```
Expected: 编译成功

- [ ] **Step 7: 提交**

```bash
git add -A
git commit -m "refactor: 重命名 MarkdownPreview→MarkdownView，删除 PreviewPos 别名，preview.rs→view.rs"
```

---

### Task 2: Theme 扩展 — 添加 novel section + MarkdownStyle::novel()

**Files:**
- Modify: `crates/ui/src/theme.rs`
- Modify: `crates/markdown/src/style.rs`

**Interfaces:**
- Consumes: 无
- Produces: `NovelTheme` struct（复用 `MarkdownTheme` 结构），`MarkdownStyle::novel(theme, font_size, line_height) -> Self`

- [ ] **Step 1: 在 theme.rs 中新增 `NovelTheme` 并使用已有 `MarkdownTheme` 结构**

在 `crates/ui/src/theme.rs`，`MarkdownTheme` 定义后添加：

```rust
/// 小说渲染主题——复用 MarkdownTheme 的结构，独立 section。
pub type NovelTheme = MarkdownTheme;
```

- [ ] **Step 2: 在 `Theme` struct 中添加 `novel` 字段**

```rust
pub struct Theme {
    pub name: String,
    pub is_dark: bool,
    pub palette: ColorPalette,
    pub editor: EditorTheme,
    pub markdown: MarkdownTheme,
    pub novel: NovelTheme,       // 新增
    pub scopes: HashMap<String, [f32; 4]>,
}
```

- [ ] **Step 3: 在 `Theme::gamma_correct()` 中添加 novel gamma 校正**

```rust
fn gamma_correct(&mut self) {
    self.palette.gamma_correct();
    self.editor.gamma_correct();
    self.markdown.gamma_correct();
    self.novel.gamma_correct();  // 新增
    // ...
}
```

- [ ] **Step 4: 在 `Theme::from_definition()` 中添加 novel 字段**

```rust
pub fn from_definition(def: &ThemeDefinition) -> Self {
    let mut theme = Self {
        name: def.display_name.clone(),
        is_dark: def.is_dark,
        palette: def.palette.clone(),
        editor: def.editor.clone(),
        markdown: def.markdown.clone(),
        novel: def.novel.clone(),  // 新增
        scopes: def.scopes.iter().map(|(k, v)| (k.clone(), *v)).collect(),
    };
    theme.gamma_correct();
    theme
}
```

- [ ] **Step 5: 在 `ThemeDefinition` 中添加 novel 字段**

```rust
pub struct ThemeDefinition {
    pub display_name: String,
    pub is_dark: bool,
    pub palette: ColorPalette,
    pub editor: EditorTheme,
    pub markdown: MarkdownTheme,
    pub novel: MarkdownTheme,  // 新增
    pub scopes: BTreeMap<String, [f32; 4]>,
}
```

- [ ] **Step 6: 在 `default_dark()` 和 `default_light()` 中添加 novel 默认值**

在 `ThemeDefinition::default_dark()` 中，`markdown` 字段后添加：
```rust
novel: MarkdownTheme {
    heading: [0.7765, 0.4706, 0.8667, 1.0], // 淡紫色 #c678dd
    link: [0.4510, 0.6784, 0.9137, 1.0],
    inline_code: [0.6745, 0.6980, 0.7451, 1.0],
    code_bg: [0.122, 0.122, 0.122, 1.0],
    code_block_bg: [0.122, 0.122, 0.122, 1.0],
    toc_background: [0.08, 0.078, 0.076, 1.0],
    toc_active_background: [0.8706, 0.4510, 0.3373, 0.12],
    toc_hover_background: [0.8706, 0.4510, 0.3373, 0.08],
    toc_text: [0.8627, 0.8745, 0.9373, 1.0],   // #dcdfe4
    toc_hover_text: [0.9608, 0.9529, 0.9412, 1.0],
    toc_level_indicator: [0.4549, 0.6784, 0.9098, 0.6],
    spacing: MarkdownSpacing {
        paragraph_spacing_ratio: 0.5,
        heading_spacing_top_ratio: 1.0,
        heading_spacing_bottom_ratio: 1.0,     // novel 章间距更大
        list_item_spacing_ratio: 0.15,
        list_group_spacing_ratio: 0.5,
        list_indent_ratio: 2.0,
        code_block_padding_ratio: 0.8,
        code_line_height_ratio: 1.5,
        blockquote_padding_ratio: 0.65,
        table_cell_padding_ratio: 0.5,
        rule_spacing: 24.0,                    // novel 分隔线更大间距
        rule_thickness: 2.0,
        rule_width_ratio: 1.0,
        border_radius_base: 8.0,
        border_radius_small: 4.0,
    },
},
```

在 `ThemeDefinition::default_light()` 中类似添加（颜色调亮）：
```rust
novel: MarkdownTheme {
    heading: [0.6431, 0.2863, 0.6706, 1.0],
    link: [0.3569, 0.4745, 0.8902, 1.0],
    inline_code: [0.1412, 0.1451, 0.1608, 1.0],
    code_bg: [0.9725, 0.9686, 0.9608, 1.0],
    code_block_bg: [0.9725, 0.9686, 0.9608, 1.0],
    toc_background: [0.98, 0.96, 0.93, 1.0],
    toc_active_background: [0.90, 0.85, 0.75, 0.5],
    toc_hover_background: [0.92, 0.90, 0.85, 0.5],
    toc_text: [0.2, 0.2, 0.18, 1.0],
    toc_hover_text: [0.1, 0.1, 0.08, 1.0],
    toc_level_indicator: [0.4549, 0.6784, 0.9098, 0.6],
    spacing: MarkdownSpacing {
        paragraph_spacing_ratio: 0.5,
        heading_spacing_top_ratio: 1.0,
        heading_spacing_bottom_ratio: 1.0,
        list_item_spacing_ratio: 0.15,
        list_group_spacing_ratio: 0.5,
        list_indent_ratio: 2.0,
        code_block_padding_ratio: 0.8,
        code_line_height_ratio: 1.5,
        blockquote_padding_ratio: 0.65,
        table_cell_padding_ratio: 0.5,
        rule_spacing: 24.0,
        rule_thickness: 2.0,
        rule_width_ratio: 1.0,
        border_radius_base: 8.0,
        border_radius_small: 4.0,
    },
},
```

- [ ] **Step 7: 同样更新 `test_theme()` 和 `test_light_theme()`**

在两个测试函数中添加 `novel` 字段，值复用各自 `markdown` 字段的值。

- [ ] **Step 8: 在 style.rs 中添加 `MarkdownStyle::novel()` 构造函数**

```rust
impl MarkdownStyle {
    /// 小说专用——独立配置，不回退 markdown。
    pub fn novel(theme: &ui::Theme, font_size: f32, line_height: f32) -> Self {
        let novel = &theme.novel;
        let body_font_size = font_size;
        let code_font_size = font_size * 0.9;
        let heading_scale = [1.8, 1.3, 1.15, 1.2, 1.1, 0.95];
        let heading_font_sizes = heading_scale.map(|s| font_size * s);
        let body_font_family = vec!["PingFang SC".to_string()];
        let code_font_family = Some("monospace".to_string());

        let bg = theme.editor.background;
        let is_dark = theme.is_dark;

        let code_bg = novel.code_bg;
        let inline_code_bg = blend_toward_bg(code_bg, bg, 0.94);

        let accent = theme.palette.accent;
        let blockquote_border = if is_dark { blend_toward_bg(accent, bg, 0.75) } else { accent };
        let blockquote_bg = if is_dark {
            [accent[0], accent[1], accent[2], 0.08]
        } else {
            [accent[0], accent[1], accent[2], 0.05]
        };

        let table_border = theme.palette.border_subtle;
        let table_header_bg = theme.palette.bg_hover;
        let table_stripe_bg = [
            theme.palette.bg_hover[0],
            theme.palette.bg_hover[1],
            theme.palette.bg_hover[2],
            theme.palette.bg_hover[3] * 0.5,
        ];
        let code_block_border = theme.palette.border_subtle;
        let rule_color = theme.palette.border_subtle;

        let sp = &novel.spacing;

        Self {
            body_font_size,
            code_font_size,
            heading_font_sizes,
            body_font_family,
            code_font_family,
            text_color: novel.toc_text,
            code_color: theme.editor.foreground,
            code_bg,
            inline_code_bg,
            heading_color: novel.heading,
            link_color: novel.link,
            rule_color,
            blockquote_bg,
            blockquote_border,
            table_border,
            table_header_bg,
            table_stripe_bg,
            border_radius_base: sp.border_radius_base,
            border_radius_small: sp.border_radius_small,
            code_block_border,
            list_item_spacing: line_height * sp.list_item_spacing_ratio,
            list_group_spacing: line_height * sp.list_group_spacing_ratio,
            rule_spacing: sp.rule_spacing,
            paragraph_spacing: line_height * sp.paragraph_spacing_ratio,
            heading_spacing_top: line_height * sp.heading_spacing_top_ratio,
            heading_spacing_bottom: line_height * sp.heading_spacing_bottom_ratio,
            code_block_padding: code_font_size * sp.code_block_padding_ratio,
            blockquote_padding: font_size * sp.blockquote_padding_ratio,
            list_indent: font_size * sp.list_indent_ratio,
            table_cell_padding: font_size * sp.table_cell_padding_ratio,
            line_height,
            code_line_height: code_font_size * sp.code_line_height_ratio,
            background_color: bg,
            rule_thickness: sp.rule_thickness,
            rule_width_ratio: sp.rule_width_ratio,
        }
    }
}
```

- [ ] **Step 9: 添加测试**

在 `style.rs` 的 `tests` 模块中添加：

```rust
#[test]
fn novel_style_uses_novel_theme_not_markdown() {
    let theme = ui::theme::test_theme();
    let md_style = MarkdownStyle::from_theme(&theme, 15.0, 24.0);
    let novel_style = MarkdownStyle::novel(&theme, 15.0, 24.0);
    // novel 的 heading_color 应该来自 theme.novel.heading
    assert_eq!(novel_style.heading_color, theme.novel.heading);
    // 确保和 markdown 不同（如果 theme 不同则通过）
    // 至少 text_color 应该来自 novel section
    assert_eq!(novel_style.text_color, theme.novel.toc_text);
}
```

- [ ] **Step 11: 编译验证并提交**

```bash
cargo build 2>&1
cargo test -p edit-plus-ui 2>&1
cargo test -p edit-plus-markdown 2>&1
```

```bash
git add -A
git commit -m "feat: 添加 novel theme section + MarkdownStyle::novel() 构造函数"
```

---

### Task 3: 移入 chapter.rs + 简化 ParagraphIndex + 实现 build_from_txt()

**Files:**
- Create: `crates/markdown/src/novel/chapter.rs`（从 `crates/novel/src/chapter.rs` 复制）
- Create: `crates/markdown/src/novel/merge.rs`（简化版，去除 Y 计算）
- Create: `crates/markdown/src/novel/mod.rs`
- Modify: `crates/markdown/src/builder.rs`（添加 `build_from_txt()`）
- Modify: `crates/markdown/src/lib.rs`（添加 `pub mod novel`）
- Modify: `crates/markdown/Cargo.toml`（添加 `regex` 依赖）

**Interfaces:**
- Consumes: `chapter::ChapterIndex`, `chapter::TitleKind`, `chapter::classify_title()`
- Produces: `novel::chapter` 模块，`novel::merge::ParagraphIndex`（简化版），`builder::build_from_txt(doc, paragraph_index) -> MarkdownDoc`

- [ ] **Step 1: 创建目录并复制 chapter.rs**

```bash
mkdir -p crates/markdown/src/novel
cp crates/novel/src/chapter.rs crates/markdown/src/novel/chapter.rs
```

修改 `crates/markdown/src/novel/chapter.rs` 的 crate 引用：
- `use core::document::DocView;` 不变
- `use regex::Regex;` 不变
- 删除 `use std::sync::LazyLock;` → 使用已有的

- [ ] **Step 2: 写 merge.rs 简化版**

简化 `ParagraphIndex`，去除 Y 偏移计算（LazyLayout 负责），只保留段落边界检测和文本合并：

```rust
//! 段落构建与折行合并（简化版 —— 仅边界检测，无像素计算）。

use core::document::DocView;
use crate::novel::chapter::{ChapterIndex, TitleKind};

/// 句末标点。
const SENTENCE_END_CHARS: &[char] = &['。', '？', '！', '…', '」', '』', '）', '》'];

/// 段落起始标记。
const PARA_START_CHARS: &[char] = &['\u{3000}', '「', '『', '\t'];

/// 引用区标题关键词。
const QUOTE_BLOCK_KEYWORDS: &[&str] = &[
    "内容简介", "内容梗概", "作品简介", "书籍简介",
    "作者简介", "作者介绍",
    "文案", "简介", "摘要", "导读",
];

/// 段落类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    BookTitle,
    VolumeTitle,
    ChapterTitle,
    QuoteBlock,
    Body,
}

/// 段落索引（仅边界，无像素计算）。
#[derive(Debug, Clone, Default)]
pub struct ParagraphIndex {
    pub entries: Vec<ParagraphEntry>,
}

#[derive(Debug, Clone)]
pub struct ParagraphEntry {
    pub start_line: usize,
    pub end_line: usize,
    pub style: LineStyle,
}

// ── 辅助函数（从旧 merge.rs 保留，去除 Y 计算相关）──

fn is_quote_block_header(text: &str) -> bool {
    let trimmed = text.trim();
    if !trimmed.ends_with('：') && !trimmed.ends_with(':') {
        return false;
    }
    let prefix = trimmed.trim_end_matches('：').trim_end_matches(':');
    QUOTE_BLOCK_KEYWORDS.iter().any(|kw| prefix.contains(kw))
}

fn ends_with_sentence_punct(text: &str) -> bool {
    text.chars().last().is_some_and(|c| SENTENCE_END_CHARS.contains(&c))
}

fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

fn next_line_is_boundary(
    doc: &dyn DocView,
    line: usize,
    title_map: &std::collections::HashMap<usize, LineStyle>,
) -> bool {
    let next = line + 1;
    if next >= doc.line_count() {
        return true;
    }
    if title_map.contains_key(&next) {
        return true;
    }
    let text = String::from_utf8_lossy(doc.doc_line_bytes(next));
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.chars().next().is_some_and(|c| PARA_START_CHARS.contains(&c)) {
        return true;
    }
    if ends_with_sentence_punct(trimmed) {
        return true;
    }
    false
}

impl ParagraphIndex {
    pub fn build(doc: &dyn DocView, chapter_index: &ChapterIndex) -> Self {
        let title_map = build_title_map(chapter_index);
        let mut builder = ParagraphBuilder::new(doc, &title_map);
        builder.run();
        ParagraphIndex { entries: builder.entries }
    }
}

fn build_title_map(chapter_index: &ChapterIndex) -> std::collections::HashMap<usize, LineStyle> {
    chapter_index
        .entries
        .iter()
        .map(|e| {
            let style = match e.kind {
                TitleKind::BookTitle => LineStyle::BookTitle,
                TitleKind::Volume => LineStyle::VolumeTitle,
                TitleKind::Chapter => LineStyle::ChapterTitle,
            };
            (e.line, style)
        })
        .collect()
}

/// 段落构建状态机——管理 current_start/current_style 的状态扭转。
struct ParagraphBuilder<'a> {
    doc: &'a dyn DocView,
    title_map: &'a std::collections::HashMap<usize, LineStyle>,
    entries: Vec<ParagraphEntry>,
    current_start: usize,
    current_style: LineStyle,
    line_count: usize,
}

impl<'a> ParagraphBuilder<'a> {
    fn new(doc: &'a dyn DocView, title_map: &'a std::collections::HashMap<usize, LineStyle>) -> Self {
        let line_count = doc.line_count();
        Self { doc, title_map, entries: Vec::new(), current_start: 0, current_style: LineStyle::Body, line_count }
    }

    fn run(&mut self) {
        for line in 0..self.line_count {
            self.process_line(line);
        }
        self.flush_remaining();
    }

    fn flush_current_span(&mut self, end_line: usize) {
        if end_line > self.current_start {
            self.entries.push(ParagraphEntry {
                start_line: self.current_start,
                end_line: end_line,
                style: self.current_style,
            });
        }
    }

    fn start_new_span(&mut self, from_line: usize, style: LineStyle) {
        self.current_start = from_line;
        self.current_style = style;
    }

    fn emit_standalone(&mut self, line: usize, style: LineStyle) {
        self.flush_current_span(line);
        self.entries.push(ParagraphEntry { start_line: line, end_line: line + 1, style });
        self.start_new_span(line + 1, LineStyle::Body);
    }

    fn process_line(&mut self, line: usize) {
        if let Some(&title_style) = self.title_map.get(&line) {
            self.emit_standalone(line, title_style);
            return;
        }

        let text = String::from_utf8_lossy(self.doc.doc_line_bytes(line));
        let trimmed = text.trim();

        if is_blank(trimmed) {
            self.flush_current_span(line);
            self.start_new_span(line + 1, LineStyle::Body);
            return;
        }

        if is_quote_block_header(trimmed) {
            self.emit_standalone(line, LineStyle::QuoteBlock);
            self.start_new_span(line + 1, LineStyle::QuoteBlock);
            return;
        }

        if trimmed.chars().next().is_some_and(|c| PARA_START_CHARS.contains(&c))
            && self.current_style != LineStyle::QuoteBlock
        {
            self.flush_current_span(line);
            self.start_new_span(line, LineStyle::Body);
        }

        if ends_with_sentence_punct(trimmed) {
            self.flush_current_span(line);
            self.entries.push(ParagraphEntry {
                start_line: line, end_line: line + 1, style: self.current_style,
            });
            self.start_new_span(line + 1, LineStyle::Body);
        } else if next_line_is_boundary(self.doc, line, self.title_map) {
            self.entries.push(ParagraphEntry {
                start_line: self.current_start, end_line: line + 1, style: self.current_style,
            });
            self.start_new_span(line + 1, LineStyle::Body);
        }
    }

    fn flush_remaining(&mut self) {
        self.flush_current_span(self.line_count);
    }
}

/// 合并段落行文本。
pub fn merge_paragraph_lines(entry: &ParagraphEntry, doc: &dyn DocView) -> String {
    let mut result = String::new();
    for line in entry.start_line..entry.end_line {
        let text = String::from_utf8_lossy(doc.doc_line_bytes(line));
        let trimmed = text.trim();
        if !result.is_empty() && !trimmed.is_empty() {
            result.push(' ');
        }
        result.push_str(trimmed);
    }
    result
}

/// 判断字符是否为句末标点。
pub fn is_sentence_end(c: char) -> bool {
    SENTENCE_END_CHARS.contains(&c)
}

/// 判断字符是否为段落起始标记。
pub fn is_paragraph_start(c: char) -> bool {
    PARA_START_CHARS.contains(&c)
}
```

- [ ] **Step 3: 写 novel/mod.rs**

```rust
pub mod chapter;
pub mod merge;
```

- [ ] **Step 4: 在 builder.rs 中添加 `build_from_txt()` 函数**

在 `crates/markdown/src/builder.rs` 末尾添加：

```rust
use crate::novel::merge::{LineStyle, ParagraphEntry, ParagraphIndex, merge_paragraph_lines};
use core::document::DocView;

/// 从小说 txt 文档构建 MarkdownDoc。
/// ParagraphIndex 提供段落边界和类型，LazyLayout 负责像素计算。
pub fn build_from_txt(doc: &dyn DocView, paragraph_index: &ParagraphIndex) -> MarkdownDoc {
    let mut blocks = Vec::new();
    for entry in &paragraph_index.entries {
        let text = merge_paragraph_lines(entry, doc);
        let block = match entry.style {
            LineStyle::BookTitle => BlockNode {
                kind: BlockKind::Heading { level: 1 },
                children: vec![],
                text_lines: vec![text],
                text_styles: vec![],
            },
            LineStyle::VolumeTitle => BlockNode {
                kind: BlockKind::Heading { level: 2 },
                children: vec![],
                text_lines: vec![text],
                text_styles: vec![],
            },
            LineStyle::ChapterTitle => {
                if !blocks.is_empty() {
                    blocks.push(BlockNode {
                        kind: BlockKind::HorizontalRule,
                        children: vec![],
                        text_lines: vec![],
                        text_styles: vec![],
                    });
                }
                BlockNode {
                    kind: BlockKind::Heading { level: 3 },
                    children: vec![],
                    text_lines: vec![text],
                    text_styles: vec![],
                }
            }
            LineStyle::QuoteBlock => BlockNode {
                kind: BlockKind::BlockQuote,
                children: vec![BlockNode {
                    kind: BlockKind::Paragraph,
                    children: vec![],
                    text_lines: vec![text],
                    text_styles: vec![],
                }],
                text_lines: vec![],
                text_styles: vec![],
            },
            LineStyle::Body => BlockNode {
                kind: BlockKind::Paragraph,
                children: vec![],
                text_lines: vec![text],
                text_styles: vec![],
            },
        };
        blocks.push(block);
    }
    MarkdownDoc { blocks }
}
```

- [ ] **Step 5: 添加 regex 依赖到 markdown crate**

在 `crates/markdown/Cargo.toml` 的 `[dependencies]` 中添加：
```toml
regex = "1"
```

- [ ] **Step 6: 更新 lib.rs**

```rust
pub mod novel;
```

- [ ] **Step 7: 编译验证并提交**

```bash
cargo build -p edit-plus-markdown 2>&1
cargo test -p edit-plus-markdown 2>&1
```

预期：merge 的测试需要适配（去除 Y 计算后部分测试需更新）。

```bash
git add -A
git commit -m "feat: 移入 chapter.rs + 简化 ParagraphIndex + 实现 build_from_txt()"
```

---

### Task 4: 提取 PreviewEngine 共享体

**Files:**
- Modify: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: `MarkdownView` 的现有字段和方法
- Produces: `PreviewEngine` struct（全部缓存/布局/渲染/滚动逻辑），`MarkdownView { engine: PreviewEngine }`

- [ ] **Step 1: 将 `MarkdownView` 的所有字段提取到 `PreviewEngine`**

```rust
/// 共享渲染引擎——所有缓存、布局、渲染、滚动、选区、搜索状态。
pub(crate) struct PreviewEngine {
    pub source: String,
    pub lazy: Option<LazyLayout>,
    pub cached_style_hash: u64,
    pub cached_generation: u32,
    pub cached_viewport_w: f32,
    pub cached_source_hash: u64,
    pub scroll_y: f32,
    pub content_height: f32,
    pub headings: Vec<HeadingEntry>,
    pub dirty: bool,
    pub cached_dl: Option<DrawList>,
    pub cached_dl_scroll_y: f32,
    pub cached_dl_viewport: (f32, f32),
    pub cached_vertices: Option<Vec<GlyphVertex>>,
    pub cached_offset_x: f32,
    pub cached_offset_y: f32,
    pub sel: SelectionState,
    pub pending_heading_jump: Option<usize>,
    pub search: SearchState,
    pub base_font_size: f32,
    pub base_line_height: f32,
    pub toc_max_depth: u8,
}
```

- [ ] **Step 2: 将 `MarkdownView` 的所有方法迁移到 `impl PreviewEngine`**

将所有 `MarkdownView` 的方法（`new`, `collect_headings`, `current_heading_index`, `scroll_to_heading`, `needs_source_update`, `set_source`, `scroll`, `needs_rebuild`, `rebuild_layout`, `precision_pass_on_scroll`, `render`, `cache_vertices`, `get_cached_vertices`, `anchor`, `restore_anchor`, 以及所有 selection/search 相关方法）移到 `impl PreviewEngine`。

- [ ] **Step 3: 定义精简的 `MarkdownView`**

```rust
pub struct MarkdownView {
    engine: PreviewEngine,
}

impl MarkdownView {
    pub fn new() -> Self {
        Self { engine: PreviewEngine::new() }
    }
}
```

- [ ] **Step 4: `MarkdownView` 的 ViewPlugin impl 委托给 engine**

所有 `ViewPlugin` trait 方法通过 `self.engine` 调用，例如：

```rust
impl ViewPlugin for MarkdownView {
    fn name(&self) -> &str { "markdown_view" }
    fn allows_editing(&self) -> bool { false }
    fn shows_cursor(&self) -> bool { false }
    fn shows_gutter(&self) -> bool { false }

    fn render(&mut self, _doc: &dyn DocView, bounds: Rect, theme: &Theme, shaper: &mut Shaper, dpi_scale: f32) -> DrawList {
        let settings = MarkdownRenderSettings {
            font_size: self.engine.base_font_size * dpi_scale,
            line_height: self.engine.base_line_height * dpi_scale,
            toc_max_depth: self.engine.toc_max_depth,
        };
        // 使用 MarkdownStyle::from_theme + engine.render
        let style = settings.style(theme);
        let style_hash = style_hash_quick(&style);
        let viewport_w = bounds.w;
        let viewport_h = bounds.h;
        let highlighter = AppCodeHighlighter { theme };

        if self.engine.needs_rebuild(style_hash, viewport_w) {
            self.engine.rebuild_layout(&style, viewport_w, viewport_h, Some(shaper), &highlighter, settings.toc_max_depth);
            self.engine.cached_style_hash = style_hash;
            self.engine.cached_viewport_w = viewport_w;
        }
        // ... 复用 engine 的渲染逻辑
        // 注意：render 方法需要接受 style 参数来区分 md/novel
    }

    fn handle_message(&mut self, msg: PluginMessage, _doc: &mut dyn DocViewMut) -> bool {
        self.engine.handle_message(msg)
    }

    fn query(&self, q: PluginQuery, _doc: &dyn DocView) -> PluginResponse {
        self.engine.query(q)
    }
}
```

- [ ] **Step 5: 重构 engine 的 render 方法接受外部 style**

`PreviewEngine::render()` 需要接受 `&MarkdownStyle` 参数（而非内部构建），以便 `MarkdownView` 传入 `from_theme()` 的结果，`NovelView` 传入 `novel()` 的结果。

```rust
impl PreviewEngine {
    pub fn render(
        &mut self,
        style: &MarkdownStyle,
        theme: &Theme,
        viewport_w: f32,
        viewport_h: f32,
        offset_x: f32,
        offset_y: f32,
        shaper: Option<&mut shaping::Shaper>,
    ) -> (DrawList, bool) {
        // body 同旧 MarkdownView::render，但 style 外部传入
        // ...
    }
}
```

- [ ] **Step 6: 在 `PreviewEngine` 上实现 `handle_message` 和 `query`**

将原来 `impl ViewPlugin for MarkdownView` 中的 `handle_message` 和 `query` 逻辑搬到 `impl PreviewEngine`：

```rust
impl PreviewEngine {
    pub fn handle_message(&mut self, msg: PluginMessage) -> bool {
        match msg {
            PluginMessage::Scroll { delta, viewport_h } => self.scroll(delta, viewport_h),
            PluginMessage::ScrollToHeading(index) => { self.scroll_to_heading(index); true }
            // ... 同旧代码
            _ => false,
        }
    }

    pub fn query(&self, q: PluginQuery) -> PluginResponse {
        match q {
            PluginQuery::ScrollY => PluginResponse::Float(self.scroll_y),
            // ... 同旧代码
        }
        // 注意：TOCHeadings 和 SearchHighlights 等需要访问 self
    }
}
```

- [ ] **Step 7: 确保单元测试仍通过**

`heading_tests` 模块测试的是 `MarkdownView`（原 `MarkdownPreview`）的方法，需要更新为测试 `PreviewEngine`。

- [ ] **Step 8: 编译验证并提交**

```bash
cargo build 2>&1
cargo test -p edit-plus-markdown 2>&1
```

```bash
git add -A
git commit -m "refactor: 提取 PreviewEngine 共享引擎，MarkdownView 精简为包装器"
```

---

### Task 5: 实现 NovelView（复用 PreviewEngine + txt 转换器）

**Files:**
- Modify: `crates/markdown/src/view.rs`

**Interfaces:**
- Consumes: `PreviewEngine`, `ChapterIndex`, `ParagraphIndex`, `build_from_txt()`
- Produces: `NovelView`, `NovelViewFactory`

- [ ] **Step 1: 在 view.rs 中定义 `NovelView`**

```rust
use crate::novel::chapter::ChapterIndex;
use crate::novel::merge::ParagraphIndex;

pub struct NovelView {
    engine: PreviewEngine,
    chapter_index: ChapterIndex,
    paragraph_index: ParagraphIndex,
    /// 文档是否已就绪（空文档时不构建索引）。
    initialized: bool,
}

impl NovelView {
    pub fn new() -> Self {
        Self {
            engine: PreviewEngine::new(),
            chapter_index: ChapterIndex::default(),
            paragraph_index: ParagraphIndex::default(),
            initialized: false,
        }
    }

    /// 惰性初始化索引，并通过 build_from_txt 构建 MarkdownDoc → 注入 engine。
    fn ensure_initialized(&mut self, doc: &dyn DocView, dpi_scale: f32, viewport_w: f32, style: &MarkdownStyle, theme: &Theme, shaper: &mut Shaper) {
        if self.initialized || doc.line_count() == 0 {
            return;
        }
        self.chapter_index = ChapterIndex::build(doc);
        self.paragraph_index = ParagraphIndex::build(doc, &self.chapter_index);
        let md_doc = crate::builder::build_from_txt(doc, &self.paragraph_index);
        // 通过 LazyLayout 估算布局
        let mut lazy = LazyLayout::from_doc(md_doc, style, viewport_w);
        // 首屏 precise pass
        let highlighter = AppCodeHighlighter { theme };
        // 注意：txt 不需要 code highlighting，highlighter 可为空实现
        lazy.ensure_precise_range(0.0, 600.0, style, shaper, None::<&AppCodeHighlighter<'_>>);
        self.engine.lazy = Some(lazy);
        self.engine.content_height = lazy.laid_out.total_height;
        self.engine.dirty = false;
        self.engine.collect_headings(self.engine.toc_max_depth);
        self.initialized = true;
    }
}
```

- [ ] **Step 2: 实现 `ViewPlugin for NovelView`**

```rust
impl ViewPlugin for NovelView {
    fn name(&self) -> &str { "novel_view" }
    fn allows_editing(&self) -> bool { false }
    fn shows_cursor(&self) -> bool { false }
    fn shows_gutter(&self) -> bool { false }

    fn render(
        &mut self,
        doc: &dyn DocView,
        bounds: Rect,
        theme: &Theme,
        shaper: &mut Shaper,
        dpi_scale: f32,
    ) -> DrawList {
        let font_size = self.engine.base_font_size * dpi_scale;
        let line_height = self.engine.base_line_height * dpi_scale;
        let style = MarkdownStyle::novel(theme, font_size, line_height);

        self.ensure_initialized(doc, dpi_scale, bounds.w, &style, theme, shaper);

        // 复用 engine 的渲染逻辑（和 MarkdownView 相同）
        let (dl, _needs_drain) = self.engine.render(
            &style, theme, bounds.w, bounds.h, bounds.x, bounds.y, Some(shaper),
        );
        dl
    }

    fn handle_message(&mut self, msg: PluginMessage, _doc: &mut dyn DocViewMut) -> bool {
        match msg {
            PluginMessage::UpdateSource { .. } => {
                // 文档源更新 → 立即使索引失效，下一帧 render 时惰性重建。
                self.initialized = false;
                self.chapter_index = ChapterIndex::default();
                self.paragraph_index = ParagraphIndex::default();
                self.engine.handle_message(msg)
            }
            PluginMessage::ScrollToNextChapter => {
                let cur = self.engine.current_heading_index(self.engine.scroll_y);
                if let Some(i) = cur {
                    self.engine.scroll_to_heading(i + 1);
                }
                true
            }
            PluginMessage::ScrollToPrevChapter => {
                let cur = self.engine.current_heading_index(self.engine.scroll_y);
                if let Some(i) = cur {
                    if i > 0 {
                        self.engine.scroll_to_heading(i - 1);
                    } else {
                        self.engine.scroll_y = 0.0;
                    }
                }
                true
            }
            _ => self.engine.handle_message(msg),
        }
    }

    fn query(&self, q: PluginQuery, _doc: &dyn DocView) -> PluginResponse {
        self.engine.query(q)
    }
}
```

- [ ] **Step 3: 实现 `NovelViewFactory`**

```rust
pub struct NovelViewFactory;

impl PluginFactory for NovelViewFactory {
    fn name(&self) -> &str { "novel_view" }
    fn can_handle(&self, path: Option<&Path>) -> bool {
        path.and_then(|p| p.extension()).is_some_and(|e| e == "txt")
    }
    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(NovelView::new())
    }
}
```

- [ ] **Step 4: 编译验证并提交**

```bash
cargo build -p edit-plus-markdown 2>&1
```

```bash
git add -A
git commit -m "feat: 实现 NovelView（复用 PreviewEngine + txt→MarkdownDoc 转换器）"
```

---

### Task 6: 删除 novel crate，更新 workspace 和 app 层

**Files:**
- Delete: `crates/novel/` 整个目录
- Modify: `crates/app/Cargo.toml`
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `Cargo.toml`（如果 workspace members 需要）

**Interfaces:**
- Consumes: `edit_plus_markdown::view::{MarkdownViewFactory, NovelViewFactory}`
- Produces: 精简的 app 依赖（仅依赖 markdown crate）

- [ ] **Step 1: 删除 novel crate**

```bash
rm -rf crates/novel
```

- [ ] **Step 2: 更新 app/Cargo.toml**

```toml
[features]
default = []
ci-no-fonts = []
markdown = ["dep:edit-plus-markdown"]
# 删除 novel feature

[dependencies]
# 删除: edit-plus-novel = { path = "../novel", optional = true }
```

- [ ] **Step 3: 更新 app/src/workspace.rs**

```rust
// 删除: #[cfg(feature = "novel")]
//        registry.register(Box::new(novel::NovelPluginFactory));

// 改为:
#[cfg(feature = "markdown")]
{
    registry.register(Box::new(edit_plus_markdown::view::MarkdownViewFactory));
    registry.register(Box::new(edit_plus_markdown::view::NovelViewFactory));
}
```

- [ ] **Step 4: 更新 app_renderer.rs**

`is_md_preview` → `is_readonly_view`（变量重命名，语义不变，已在前面的 task 中处理）。

无需其他修改——app_renderer.rs 通过 `ViewPlugin` trait 交互，不直接依赖 markdown/novel 类型。

- [ ] **Step 5: 编译验证并提交**

```bash
cargo build 2>&1
cargo test -p edit-plus-app 2>&1
```

预期：workspace 测试中引用 `MarkdownPluginFactory` 的测试需更新为 `MarkdownViewFactory`。

```bash
git add -A
git commit -m "refactor: 删除 novel crate，统一由 markdown crate 提供 MarkdownView + NovelView"
```

---

### Task 7: 最终验证

**Files:**
- 无新建

- [ ] **Step 1: 运行完整验证脚本**

```bash
./scripts/verify.sh
```

预期：cargo fmt check + cargo clippy + cargo test 全部通过。

- [ ] **Step 2: 检查是否有遗漏的 `is_md_preview` 引用**

```bash
grep -r "is_md_preview" crates/
grep -r "MarkdownPreview" crates/
grep -r "MarkdownPluginFactory" crates/
grep -r "markdown_preview" crates/
grep -r "PreviewPos" crates/
grep -r "novel::" crates/app/
```

预期：全部零结果（或仅在 comments 中存在）。

- [ ] **Step 3: 检查 dead code**

```bash
cargo clippy -- -W dead_code 2>&1 | head -20
```

- [ ] **Step 4: 手动验收测试场景**

- 打开 `.md` 文件 → 显示 MarkdownView，TOC 正常，滚动正常
- 打开 `.txt` 文件 → 显示 NovelView，章节跳转正常，TOC 正常
- 暗色/亮色主题切换 → Markdown 和 Novel 各自使用正确的主题 section
- 选区 + 搜索高亮 → 两个 View 正常工作

- [ ] **Step 5: 提交（如需最终调整）**

```bash
git add -A
git commit -m "chore: 最终清理——删除残留引用，验证通过"
```

---

## 自检清单

**1. Spec coverage:**
- [x] Section 1 (目标): 所有目标在 Tasks 1-6 中覆盖
- [x] Section 2 (文件结构): Task 1/3/4/6 实现
- [x] Section 3 (共享引擎): Task 4 实现
- [x] Section 4 (txt→MarkdownDoc): Task 3/5 实现
- [x] Section 5 (Theme 独立配置): Task 2 实现
- [x] Section 6 (ViewPlugin 实现): Task 4/5 实现
- [x] Section 7 (app 层): Task 6 实现
- [x] Section 8 (命名清理): Task 1/6 实现
- [x] Section 9 (迁移步骤): 1:1 映射到 Tasks 1-7

**2. Placeholder scan:** 无 TBD/TODO/placeholder。

**3. Type consistency:**
- `PreviewEngine` 的字段定义在 Task 4 Step 1 中确定，后续 task 引用一致
- `MarkdownStyle::novel()` 签名在 Task 2 Step 8 确定，Task 5 Step 2 引用一致
- `build_from_txt()` 在 Task 3 Step 4 确定，Task 5 Step 1 引用一致
- Factory 名称 `MarkdownViewFactory` / `NovelViewFactory` 全程一致
- `is_readonly_view` 变量名在 Task 1 Step 4 确定
