# 小说阅读模式设计

## 概述

为编辑器新增小说阅读模式，针对 txt 文件提供：
- 章节标题识别与差异化样式（大字号、粗体）
- 老旧 txt 排版折行自动合并
- 按章节断页（视觉分隔 + 导航跳转）
- 手动切换阅读/编辑模式，per-file 记忆

## 核心架构

```
                    DocView + DocViewMut (core)
                           ↑
              implements   │  uses as param
          ┌────────────────┤
          │                │
    ViewPlugin (ui)  ───  PluginFactory (ui)
    PluginMessage           PluginRegistry (ui)
    PluginQuery
    PluginResponse
          ↑
          │ implements
    ┌─────┴─────┬──────────────┐
    │           │              │
EditorPlugin  MarkdownPlugin  NovelView
(app)         (markdown)      (novel crate)
```

**关键原则**：
- `ViewPlugin` 替代 `ContentPlugin`，参数用 `&dyn DocView` 替代 `&DocumentView`
- `PluginFactory` 动态注册插件，`app` 不硬编码文件类型路由
- `novel` crate 只依赖 `core + ui + shaping`，零依赖 `app`
- `PreviewPlugin` 删除，预览能力走 `PluginMessage`/`PluginQuery` 事件驱动

---

## 新增/修改 trait

### core: DocView / DocViewMut

```rust
// crates/core/src/document.rs

pub trait DocView {
    fn line_count(&self) -> usize;
    fn doc_line_bytes(&self, line: usize) -> &[u8];
    fn line_byte_offset(&self, line: usize) -> usize;
    fn line_byte_length(&self, line: usize) -> usize;
    fn scroll_y(&self) -> f32;
    fn viewport_height(&self) -> f32;
    fn is_empty(&self) -> bool { self.line_count() == 0 }
}

pub trait DocViewMut: DocView {
    fn set_scroll_y(&mut self, y: f32);
}
```

### ui: ViewPlugin + PluginFactory + PluginRegistry

```rust
// crates/ui/src/plugin.rs

pub enum PluginMessage {
    Scroll { delta: f32, viewport_h: f32 },
    ScrollToHeading(usize),
    ScrollToSearchMatch { query: String, match_case: bool, active_idx: usize },
    ScrollToNextChapter,
    ScrollToPrevChapter,
    UpdateSource { text: String, generation: u32 },
}

pub enum PluginQuery {
    ScrollY,
    ContentHeight,
    NeedsSourceUpdate(u32),
    TOCHeadings,
    CurrentHeadingIndex(f32),
    HasSelection,
    SelectedText,
    SelCursor,
    HitTest { x: f32, y: f32, offset_x: f32, offset_y: f32 },
    SelectionRange,
}

pub enum PluginResponse {
    None,
    Float(f32),
    Bool(bool),
    String(String),
    Headings(Vec<HeadingEntry>),
    Position(Option<(usize, usize)>),
    DrawList(DrawList),
}

pub trait ViewPlugin {
    fn name(&self) -> &str;
    fn render(&mut self, doc: &dyn DocView, bounds: Rect, theme: &Theme,
              shaper: &mut Shaper) -> DrawList;
    fn handle_message(&mut self, msg: PluginMessage, doc: &mut dyn DocViewMut) -> bool { false }
    fn query(&self, query: PluginQuery, doc: &dyn DocView) -> PluginResponse { PluginResponse::None }
    fn shows_cursor(&self) -> bool { true }
    fn shows_gutter(&self) -> bool { true }
    fn allows_editing(&self) -> bool { true }
}

pub trait PluginFactory: Send + Sync {
    fn name(&self) -> &str;
    fn can_handle(&self, path: Option<&Path>) -> bool;
    fn create(&self) -> Box<dyn ViewPlugin>;
}

pub struct PluginRegistry {
    factories: Vec<Box<dyn PluginFactory>>,
}

impl PluginRegistry {
    pub fn new() -> Self { Self { factories: Vec::new() } }
    pub fn register(&mut self, factory: Box<dyn PluginFactory>) { self.factories.push(factory); }

    pub fn create_for_file(&self, path: Option<&Path>) -> Box<dyn ViewPlugin> {
        for factory in &self.factories {
            if factory.can_handle(path) {
                return factory.create();
            }
        }
        // 默认：EditorPlugin（不通过工厂，硬编码 fallback）
        Box::new(EditorPlugin)
    }
}
```

### app: DocumentView 实现 DocView/DocViewMut

```rust
// crates/app/src/document_view/mod.rs

impl DocView for DocumentView { ... }
impl DocViewMut for DocumentView { ... }
```

---

## novel crate

### 结构

```
crates/novel/
  Cargo.toml           # 依赖: core, ui, shaping
  src/
    lib.rs             # NovelView, NovelPluginFactory, LineStyle
    chapter.rs         # 章节识别（纯函数）
    merge.rs           # 段落构建与折行合并（纯函数）
    render.rs          # 阅读模式渲染
```

### NovelView

```rust
pub struct NovelView {
    chapter_index: ChapterIndex,
    paragraph_index: ParagraphIndex,
    enabled: bool,
}

impl ViewPlugin for NovelView {
    fn name(&self) -> &str { "novel" }
    fn allows_editing(&self) -> bool { false }
    fn shows_cursor(&self) -> bool { false }
    fn shows_gutter(&self) -> bool { false }

    fn render(&mut self, doc: &dyn DocView, bounds: Rect, theme: &Theme,
              shaper: &mut Shaper) -> DrawList {
        if !self.enabled {
            return DrawList::new(); // 回退到编辑模式
        }
        render::render_novel(doc, &self.chapter_index, &self.paragraph_index,
                             bounds, theme, shaper, ...)
    }

    fn handle_message(&mut self, msg: PluginMessage, doc: &mut dyn DocViewMut) -> bool {
        match msg {
            PluginMessage::ScrollToNextChapter => {
                // ... 跳章逻辑
                doc.set_scroll_y(self.scroll_y);
                true
            }
            PluginMessage::ScrollToPrevChapter => { ... }
            PluginMessage::Scroll { delta, .. } => {
                self.scroll_y = (self.scroll_y + delta).max(0.0);
                doc.set_scroll_y(self.scroll_y);
                true
            }
            _ => false,
        }
    }
}

pub struct NovelPluginFactory;

impl PluginFactory for NovelPluginFactory {
    fn name(&self) -> &str { "novel" }
    fn can_handle(&self, path: Option<&Path>) -> bool {
        path.and_then(|p| p.extension()).map_or(false, |e| e == "txt")
    }
    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(NovelView::new())
    }
}
```

注意：`NovelView` 初始化时不传 `doc`（Factory 的 `create()` 无 doc 参数）。章节索引和段落索引在首次 `render()` 时惰性构建，或通过一个单独的 `init(doc: &dyn DocView)` 方法在 app 层打开文件后调用。

---

## app 集成

### 初始化

```rust
// crates/app/src/app.rs 或初始化入口

let mut registry = PluginRegistry::new();
// EditorPlugin 是默认 fallback，不注册
#[cfg(feature = "markdown")]
registry.register(Box::new(edit_plus_markdown::MarkdownPluginFactory));
#[cfg(feature = "novel")]
if settings.enable_novel_mode {
    registry.register(Box::new(edit_plus_novel::NovelPluginFactory));
}
```

### 文件打开

```rust
// crates/app/src/workspace.rs

let plugin = registry.create_for_file(Some(path));
let mut entry = DocItem { doc: dv, plugin };
// 如果 plugin 需要 init，在装载后回调
plugin.handle_message(PluginMessage::InitAfterOpen, &mut entry.doc);
```

### 模式切换

```rust
// Ctrl+Shift+R
// NovelView 内部 enabled: bool 标志
// enabled = false → render 返回空 DrawList，app 走编辑器渲染
```

### 配置

```rust
// crates/ui/src/settings.rs
#[cfg(feature = "novel")]
pub enable_novel_mode: bool,  // 默认 true
```

---

## 章节识别算法

### 正则匹配

```
/^(第[一二三四五六七八九十百千万\d]+[卷章节回]|序章|楔子|尾声|番外|Chapter\s*\d+)/u
```

### 负面排除

| 常量 | 含义 | 值 |
|------|------|-----|
| `MAX_TITLE_LENGTH` | 最大标题字节长度 | 120 |
| `MAX_PUNCT_DENSITY` | 最大标点密度 | 0.5 |
| `MAX_NON_CJK_RATIO` | 非 CJK 字符最大占比 | 0.6 |

### 输出

```rust
pub struct ChapterIndex { pub entries: Vec<ChapterEntry> }
pub struct ChapterEntry { pub line: usize, pub title: String }
```

---

## 段落识别与折行合并

状态机 O(n) 预扫描：

```
初始：新段落
每行 → 是章节标题 → 当前段落结束，新开 ChapterTitle 段落
每行 → 是空行 → 当前段落结束
每行 → 是段首标志 → 当前段落结束，新开 Body 段落
每行 → 是段尾标点 → 加入段落，段落结束
每行 → 普通行 → 加入段落
```

段尾字符：`。？！"」…）—`

段首标志：全角空格 U+3000、半角空格、`「『（《`

输出：

```rust
pub struct ParagraphIndex {
    pub entries: Vec<ParagraphEntry>,
    pub cumulative_heights: Vec<f32>,
}
pub struct ParagraphEntry {
    pub start_line: usize, pub end_line: usize, pub style: LineStyle,
}
```

---

## 渲染

`novel::render::render_novel()` 直接使用 `Shaper` + `DrawList`，视口裁剪：

- 章节标题：1.5x 字号、粗体、居中、分隔线
- 正文：默认字号、左对齐（留白 16px）
- 章节间距：标题 1.5x 行高 + 正文 1x 行高

样式参数复用 `Theme::markdown`（heading_color 等）。

---

## 章节断页

- Cmd+Down → `PluginMessage::ScrollToNextChapter`
- Cmd+Up → `PluginMessage::ScrollToPrevChapter`
- 跳转锚定：目标章节标题对齐视口顶部

---

## 不纳入范围

- 自定义阅读主题
- 阅读进度/书签
- 目录侧栏集成
- EPUB/PDF 等格式
