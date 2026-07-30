# MarkdownView & Novel 合并设计

## 1. 目标

- Novel crate 合并到 markdown crate，共享渲染管线
- `MarkdownPreview` → `MarkdownView`，消除 "preview" 只读暗示
- Novel 通过 `txt→MarkdownDoc` 转换器复用 parse→build→layout→render 管线
- Markdown 和 Novel 共用 `PluginRegistry` 注册，各自独立 Theme 配置

## 2. 文件结构

```
crates/markdown/src/
  view.rs          ← MarkdownView + NovelView + PreviewEngine (共享引擎)
  selection.rs     ← ViewPos + SelectionState + 纯函数 (已提取)
  search.rs        ← SearchState (已提取)
  builder.rs       ← MarkdownDoc::build() + build_from_txt()
  layout.rs        ← LazyLayout + char_at_x/char_x (已提取)
  render.rs        ← render_doc
  parser.rs        ← pulldown-cmark markdown 解析
  style.rs         ← MarkdownStyle + novel()
  novel/
    chapter.rs     ← ChapterIndex + classify_title() (移入)

crates/novel/      → 删除整个 crate
```

## 3. 共享引擎

`PreviewEngine` 是所有缓存、渲染、滚动、选区、搜索的共享体。两个 View 各取所需：

```rust
struct PreviewEngine {
    source: String,
    lazy: Option<LazyLayout>,
    cached_style_hash: u64,
    cached_generation: u32,
    cached_viewport_w: f32,
    cached_source_hash: u64,
    scroll_y: f32,
    content_height: f32,
    headings: Vec<HeadingEntry>,
    dirty: bool,
    cached_dl: Option<DrawList>,
    cached_dl_scroll_y: f32,
    cached_dl_viewport: (f32, f32),
    cached_vertices: Option<Vec<GlyphVertex>>,
    cached_offset_x: f32,
    cached_offset_y: f32,
    sel: SelectionState,
    pending_heading_jump: Option<usize>,
    search: SearchState,
    base_font_size: f32,
    base_line_height: f32,
    toc_max_depth: u8,
}
```

```rust
pub struct MarkdownView {
    engine: PreviewEngine,
}

pub struct NovelView {
    engine: PreviewEngine,
    chapter_index: ChapterIndex,
    paragraph_index: ParagraphIndex,
}
```

## 4. txt→MarkdownDoc 转换器

### 4.1 简化的 ParagraphIndex

只做段落边界检测 + 文本合并，不再计算 Y 偏移（LazyLayout 负责像素）：

```rust
pub struct ParagraphIndex {
    pub entries: Vec<ParagraphEntry>,
}

pub struct ParagraphEntry {
    pub start_line: usize,
    pub end_line: usize,
    pub style: LineStyle,
}
```

### 4.2 LineStyle → Block 映射

| Novel 语义 | Markdown block | 级别 |
|-----------|---------------|------|
| BookTitle | Heading { level: 1 } | 最高 |
| VolumeTitle | Heading { level: 2 } | |
| ChapterTitle | Heading { level: 3 } | 章级 |
| QuoteBlock | BlockQuote | |
| Body | Paragraph | |

章节前自动插入 HorizontalRule 作为分隔。

### 4.3 转换函数

```rust
pub fn build_from_txt(
    doc: &dyn DocView,
    paragraph_index: &ParagraphIndex,
) -> MarkdownDoc {
    let mut blocks = Vec::new();
    for entry in &paragraph_index.entries {
        let text = merge_lines(doc, entry);
        let block = match entry.style {
            LineStyle::BookTitle   => Block::Heading { level: 1, text },
            LineStyle::VolumeTitle => Block::Heading { level: 2, text },
            LineStyle::ChapterTitle => {
                if !blocks.is_empty() {
                    blocks.push(Block::HorizontalRule);
                }
                Block::Heading { level: 3, text }
            }
            LineStyle::QuoteBlock => Block::BlockQuote {
                children: vec![Block::Paragraph { text }],
            },
            LineStyle::Body => Block::Paragraph { text },
        };
        blocks.push(block);
    }
    MarkdownDoc { blocks }
}
```

## 5. Theme 独立配置

```rust
impl MarkdownStyle {
    pub fn from_theme(theme: &Theme, font_size: f32, line_height: f32) -> Self { ... }

    /// 小说专用——独立配置，不回退 markdown。
    pub fn novel(theme: &Theme, font_size: f32, line_height: f32) -> Self {
        let novel = &theme.novel;
        Self {
            text_color:          novel.text_color,
            heading_color:       novel.heading_color,
            heading_font_sizes:  [novel.h1_size, novel.h2_size, novel.h3_size],
            chapter_separator_spacing: novel.chapter_spacing,
            // 其他字段从 novel section 取值
        }
    }
}
```

Theme 文件独立 section：
```toml
[novel]
text_color = "#dcdfe4"
heading_color = "#c678dd"
h1_size = 30.0
h2_size = 24.0
h3_size = 20.0
chapter_spacing = 48.0
```

## 6. ViewPlugin 实现

两个 View 各自 `impl ViewPlugin`，通过 `PluginFactory::can_handle()` 区分：

- `MarkdownPluginFactory::can_handle()` → `.md`
- `NovelPluginFactory::can_handle()` → `.txt`

内部通过构造不同的 `MarkdownStyle`（`from_theme` vs `novel`）和不同的 `MarkdownDoc` 来源（`parse_markdown` vs `build_from_txt`）实现差异。

### 章节跳转

```rust
impl NovelView {
    fn jump_next_chapter(&mut self) {
        let current = self.engine.current_heading_index(self.engine.scroll_y);
        if let Some(i) = current {
            self.engine.scroll_to_heading(i + 1);
        }
    }
}
```

复用已有的 `current_heading_index()` 和 `scroll_to_heading()`，无需新增逻辑。

## 7. app 层

- 两个工厂通过 `PluginRegistry` 注册，与现状一致
- `app_renderer.rs` 中 `is_md_preview` 改名 `is_readonly_view`
- 零硬编码类型判断，全部通过 `ViewPlugin` trait 交互

## 8. 命名清理

| 现状 | 改造后 |
|------|--------|
| `MarkdownPreview` | `MarkdownView` |
| `preview.rs` | `view.rs` |
| `PreviewPos` (alias) | 删除 alias，统一用 `ViewPos` |
| `preview_hit_test()` 等 | 已在 `selection.rs` 改为纯函数 |
| `"markdown_preview"` | `"markdown_view"` |

## 9. 迁移步骤

1. 重命名：`MarkdownPreview`→`MarkdownView`，`preview.rs`→`view.rs`
2. 提取 `PreviewEngine`，`MarkdownView`/`NovelView` 共享
3. 简化 `ParagraphIndex`（去除 Y 计算），实现 `build_from_txt()`
4. 移入 `chapter.rs` 到 `markdown/src/novel/`
5. `NovelView` impl ViewPlugin（复用 engine + txt 转换器）
6. 实现 `MarkdownStyle::novel()`
7. 删除 `crates/novel/`，更新 workspace
8. `./scripts/verify.sh` 验证
