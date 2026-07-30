# 小说阅读模式实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为编辑器新增小说阅读模式，支持 txt 文件章节识别、折行合并、章节断页导航。

**Architecture:** 分两阶段。先解耦：`DocView` trait 进 `core`，`ViewPlugin` + `PluginFactory` + `PluginRegistry` 进 `ui`，`PreviewPlugin` 删除，一切改为事件驱动。再新增：`novel` crate 实现 `ViewPlugin` + `PluginFactory`，app 通过 feature gate 注册。

**Tech Stack:** Rust, cosmic-text shaping, wgpu rendering, regex for chapter detection

## Global Constraints

- `ViewPlugin` 参数用 `&dyn DocView`，不用 `&DocumentView`
- `PluginFactory::create()` 无 DocumentView 参数；索引惰性构建
- `novel` crate 零依赖 `app`，只依赖 `core + ui + shaping`
- `app` 不硬编码文件类型路由，全部走 `PluginRegistry`
- 千万字文件：预扫描 < 200ms，滚动 60fps
- 所有魔法值提取为语义化常量

---

## 文件变更

### 新建
- `crates/novel/Cargo.toml`
- `crates/novel/src/lib.rs`
- `crates/novel/src/chapter.rs`
- `crates/novel/src/merge.rs`
- `crates/novel/src/render.rs`

### 修改
- `crates/core/src/document.rs` — 新增 `DocView` / `DocViewMut`
- `crates/ui/src/plugin.rs` — 新增 `ViewPlugin` / `PluginFactory` / `PluginRegistry` / `PluginMessage` / `PluginQuery` / `PluginResponse`
- `crates/ui/src/lib.rs` — 导出 plugin 模块
- `crates/app/src/document_view/mod.rs` — `impl DocView/Mut for DocumentView`
- `crates/app/src/plugin.rs` — 删除（替换为 re-export `ui::plugin` 或直接删除）
- `crates/app/src/preview_plugin.rs` — 删除
- `crates/app/src/plugins/editor.rs` — `impl ViewPlugin for EditorPlugin`
- `crates/app/src/plugins/markdown.rs` — `impl ViewPlugin for MarkdownPlugin` + `impl PluginFactory`
- `crates/app/src/plugin_registry.rs` — 删除（逻辑移入 ui）
- `crates/app/src/tab.rs` — `ContentPlugin` → `ViewPlugin`
- `crates/app/src/workspace.rs` — 移除硬编码路由，用 `PluginRegistry`
- `crates/app/Cargo.toml` — novel feature + dep
- `crates/ui/src/settings.rs` — `enable_novel_mode`

### 删除
- `crates/app/src/preview_plugin.rs`
- `crates/app/src/plugin_registry.rs`
- `crates/app/src/plugin.rs`

### 所有 preview 调用点适配
- `app_renderer.rs` — `.preview().xxx()` → `.query()` / `.handle_message()`
- `app_scroll.rs`
- `app_search.rs`
- `dispatch/editor.rs`
- `dispatch/mouse.rs`

---

### Task 1: DocView trait → core

**Files:**
- Modify: `crates/core/src/document.rs`
- Modify: `crates/core/src/lib.rs`

**Produces:** `core::document::DocView`, `core::document::DocViewMut`

- [ ] **Step 1: 添加 trait**

```rust
// crates/core/src/document.rs — 在 ReadableDocument/WriteableDocument 之后追加

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

- [ ] **Step 2: 导出**

```rust
// crates/core/src/lib.rs
pub use document::{DocView, DocViewMut};
```

- [ ] **Step 3: 编译**

```bash
cargo build -p edit-plus-core
```

- [ ] **Step 4: Commit**

```bash
git add crates/core/src/document.rs crates/core/src/lib.rs
git commit -m "feat(core): add DocView and DocViewMut traits"
```

---

### Task 2: ViewPlugin + PluginFactory + PluginRegistry → ui

**Files:**
- Create/Modify: `crates/ui/src/plugin.rs`
- Modify: `crates/ui/src/lib.rs`

**Produces:** `ui::plugin::{ViewPlugin, PluginFactory, PluginRegistry, PluginMessage, PluginQuery, PluginResponse}`

- [ ] **Step 1: 定义事件与查询**

```rust
// crates/ui/src/plugin.rs

use std::path::Path;
use core::document::{DocView, DocViewMut};
use crate::core::geom::Rect;
use crate::core::paint::DrawList;
use crate::theme::Theme;
use shaping::Shaper;

/// 插件消息（命令）。
pub enum PluginMessage {
    Scroll { delta: f32, viewport_h: f32 },
    ScrollToHeading(usize),
    ScrollToSearchMatch { query: String, match_case: bool, active_idx: usize },
    ScrollToNextChapter,
    ScrollToPrevChapter,
    UpdateSource { text: String, generation: u32 },
}

/// 插件查询（同步返回数据）。
pub enum PluginQuery {
    ScrollY,
    ContentHeight,
    NeedsSourceUpdate(u32),
    TOCHeadings,
    CurrentHeadingIndex(f32),
    HasSelection,
    SelectedText,
    SelCursor,
    SelectionRange,
    HitTest { x: f32, y: f32, offset_x: f32, offset_y: f32 },
    SearchHighlights { query: String, match_case: bool, use_regex: bool, active_idx: usize, match_color: [f32; 4], inactive_color: [f32; 4] },
    SelectionHighlights([f32; 4]),
    FlatLines,
}

/// 插件响应。
pub enum PluginResponse {
    None,
    Float(f32),
    Bool(bool),
    String(String),
    Headings(Vec<HeadingEntry>),
    Position(Option<(usize, usize)>),
    DrawList(DrawList),
    FlatLines(Vec<FlatLine>),
}

/// 通用 TOC 条目。
#[derive(Debug, Clone)]
pub struct HeadingEntry {
    pub title: String,
    pub y: f32,
    pub level: u8,
}

/// 通用扁平行（用于搜索/复制）。
#[derive(Debug, Clone)]
pub struct FlatLine {
    pub text: String,
}
```

- [ ] **Step 2: 定义 ViewPlugin trait**

```rust
pub trait ViewPlugin {
    fn name(&self) -> &str;

    fn render(&mut self, doc: &dyn DocView, bounds: Rect, theme: &Theme,
              shaper: &mut Shaper) -> DrawList;

    fn handle_message(&mut self, msg: PluginMessage, doc: &mut dyn DocViewMut) -> bool {
        let _ = (msg, doc);
        false
    }

    fn query(&self, query: PluginQuery, doc: &dyn DocView) -> PluginResponse {
        let _ = (query, doc);
        PluginResponse::None
    }

    fn shows_cursor(&self) -> bool { true }
    fn shows_gutter(&self) -> bool { true }
    fn allows_editing(&self) -> bool { true }
}
```

- [ ] **Step 3: 定义 PluginFactory 和 PluginRegistry**

```rust
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

    pub fn register(&mut self, factory: Box<dyn PluginFactory>) {
        self.factories.push(factory);
    }

    /// 寻找第一个能处理该文件的工厂，找不到则返回编辑器插件。
    pub fn create_for_file(
        &self,
        path: Option<&Path>,
        editor_fallback: Box<dyn ViewPlugin>,
    ) -> Box<dyn ViewPlugin> {
        for f in &self.factories {
            if f.can_handle(path) {
                return f.create();
            }
        }
        editor_fallback
    }
}
```

- [ ] **Step 4: 在 ui/src/lib.rs 导出**

```rust
pub mod plugin;
```

- [ ] **Step 5: 编译**

```bash
cargo build -p edit-plus-ui
```

- [ ] **Step 6: Commit**

```bash
git add crates/ui/src/plugin.rs crates/ui/src/lib.rs
git commit -m "feat(ui): add ViewPlugin, PluginFactory, PluginRegistry"
```

---

### Task 3: DocumentView 实现 DocView/DocViewMut

**Files:**
- Modify: `crates/app/src/document_view/mod.rs`

**Produces:** `impl DocView for DocumentView`, `impl DocViewMut for DocumentView`

- [ ] **Step 1: 给 DocumentView 加零拷贝行读取方法**

```rust
impl DocumentView {
    fn raw_line_bytes(&self, line: usize) -> &[u8] {
        let off = self.line_byte_offset(line);
        let len = self.line_byte_length(line);
        self.tb.read_forward(off).get(..len).unwrap_or(&[])
    }
}
```

- [ ] **Step 2: 实现 DocView 和 DocViewMut**

```rust
use core::document::{DocView, DocViewMut};

impl DocView for DocumentView {
    fn line_count(&self) -> usize {
        LineIndex::line_count(&self.line_index)
    }
    fn doc_line_bytes(&self, line: usize) -> &[u8] {
        self.raw_line_bytes(line)
    }
    fn line_byte_offset(&self, line: usize) -> usize {
        self.line_index.offsets.get(line).copied().unwrap_or(0)
    }
    fn line_byte_length(&self, line: usize) -> usize {
        self.line_index.lengths.get(line).copied().unwrap_or(0)
    }
    fn scroll_y(&self) -> f32 {
        self.display.viewport.scroll_y as f32
    }
    fn viewport_height(&self) -> f32 {
        self.display.viewport.viewport_height as f32
    }
    fn is_empty(&self) -> bool {
        self.tb.is_empty()
    }
}

impl DocViewMut for DocumentView {
    fn set_scroll_y(&mut self, y: f32) {
        self.display.viewport.scroll_y = y as f64;
    }
}
```

- [ ] **Step 3: 编译验证**

```bash
cargo build -p edit-plus-app 2>&1 | head -20
# 此时 app 还有大量编译错误（旧 ContentPlugin/preview 还在），等待后续 Task 修复
```

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/document_view/mod.rs
git commit -m "refactor(app): impl DocView+DocViewMut for DocumentView"
```

---

### Task 4: 删除 PreviewPlugin，适配 EditorPlugin → ViewPlugin

**Files:**
- Delete: `crates/app/src/preview_plugin.rs`
- Delete: `crates/app/src/plugin.rs`
- Delete: `crates/app/src/plugin_registry.rs`
- Modify: `crates/app/src/plugins/editor.rs`
- Modify: `crates/app/src/plugins/mod.rs`

**Produces:** EditorPlugin 实现 ViewPlugin，旧接口清理

- [ ] **Step 1: 删除旧文件**

```bash
git rm crates/app/src/preview_plugin.rs
git rm crates/app/src/plugin.rs
git rm crates/app/src/plugin_registry.rs
```

- [ ] **Step 2: 改写 EditorPlugin**

```rust
// crates/app/src/plugins/editor.rs
use ui::plugin::ViewPlugin;
use core::document::DocView;
use shaping::Shaper;
use ui::core::geom::Rect;
use ui::core::paint::DrawList;
use ui::theme::Theme;

pub(crate) struct EditorPlugin;

impl ViewPlugin for EditorPlugin {
    fn name(&self) -> &str { "editor" }

    fn render(&mut self, doc: &dyn DocView, bounds: Rect, theme: &Theme,
              shaper: &mut Shaper) -> DrawList {
        let _ = (doc, bounds, theme, shaper);
        DrawList::new() // Phase 1 stub
    }

    fn shows_cursor(&self) -> bool { true }
    fn shows_gutter(&self) -> bool { true }
}
```

- [ ] **Step 3: 更新 plugins/mod.rs**

```rust
pub(crate) mod editor;
#[cfg(feature = "markdown")]
pub(crate) mod markdown;
```

- [ ] **Step 4: 编译检查**

```bash
cargo build -p edit-plus-app 2>&1 | head -30
# 预期：MarkdownPlugin 还有旧的 ContentPlugin/PreviewPlugin 引用，等 Task 5 修
```

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/plugins/ crates/app/src/
git commit -m "refactor(app): delete PreviewPlugin/ContentPlugin, adapt EditorPlugin to ViewPlugin"
```

---

### Task 5: 适配 MarkdownPlugin → ViewPlugin + PluginFactory

**Files:**
- Modify: `crates/app/src/plugins/markdown.rs`

**Produces:** MarkdownPlugin 实现 ViewPlugin + PluginFactory

- [ ] **Step 1: 重写 MarkdownPlugin 为 ViewPlugin**

所有 `PreviewPlugin` 中的方法映射为 `handle_message` / `query`：

```rust
use ui::plugin::{ViewPlugin, PluginFactory, PluginMessage, PluginQuery, PluginResponse};
use core::document::{DocView, DocViewMut};

impl ViewPlugin for MarkdownPlugin {
    fn name(&self) -> &str { "markdown_preview" }
    fn allows_editing(&self) -> bool { false }
    fn shows_cursor(&self) -> bool { false }
    fn shows_gutter(&self) -> bool { false }

    fn render(&mut self, doc: &dyn DocView, bounds: Rect, theme: &Theme,
              shaper: &mut Shaper) -> DrawList {
        // 原 render 逻辑，Markdown 预览不使用 doc（通过 set_source 获取内容）
        let _ = doc;
        let (dl, _) = self.preview.render(theme, bounds.w, bounds.h, bounds.x, bounds.y,
                                          self.render_settings(), shaper);
        dl
    }

    fn handle_message(&mut self, msg: PluginMessage, doc: &mut dyn DocViewMut) -> bool {
        match msg {
            PluginMessage::Scroll { delta, viewport_h } => self.preview.scroll(delta, viewport_h),
            PluginMessage::ScrollToHeading(idx) => { self.preview.scroll_to_heading(idx); true }
            PluginMessage::UpdateSource { text, generation } => { self.preview.set_source(text, generation); true }
            PluginMessage::ScrollToSearchMatch { query, match_case, active_idx } => {
                self.preview.scroll_to_search_match(&query, match_case, active_idx);
                true
            }
            _ => false,
        }
    }

    fn query(&self, query: PluginQuery, doc: &dyn DocView) -> PluginResponse {
        match query {
            PluginQuery::ScrollY => PluginResponse::Float(self.preview.scroll_y()),
            PluginQuery::ContentHeight => PluginResponse::Float(self.preview.content_height()),
            PluginQuery::NeedsSourceUpdate(gen) => PluginResponse::Bool(self.preview.needs_source_update(gen)),
            PluginQuery::TOCHeadings => PluginResponse::Headings(self.preview.headings().to_vec()),
            PluginQuery::CurrentHeadingIndex(sy) => {
                match self.preview.current_heading_index(sy) {
                    Some(i) => PluginResponse::Float(i as f32),
                    None => PluginResponse::None,
                }
            }
            PluginQuery::HasSelection => PluginResponse::Bool(self.preview.has_preview_selection()),
            PluginQuery::SelectedText => PluginResponse::String(self.preview.preview_selected_text().unwrap_or_default()),
            PluginQuery::SelCursor => PluginResponse::Position(self.preview.sel_cursor().map(|p| (p.line, p.byte))),
            PluginQuery::SelectionRange => {
                self.preview.preview_selection_range()
                    .map(|(a, b)| PluginResponse::Position(Some((a.line, a.byte))))
                    .unwrap_or(PluginResponse::Position(None))
            }
            PluginQuery::HitTest { x, y, offset_x, offset_y } => {
                self.preview.preview_hit_test(x, y, offset_x, offset_y)
                    .map(|p| PluginResponse::Position(Some((p.line, p.byte))))
                    .unwrap_or(PluginResponse::Position(None))
            }
            PluginQuery::SearchHighlights { query, match_case, use_regex, active_idx, match_color, inactive_color } => {
                PluginResponse::DrawList(self.preview.search_highlights(&query, match_case, use_regex, active_idx, match_color, inactive_color))
            }
            PluginQuery::SelectionHighlights(color) => {
                PluginResponse::DrawList(self.preview.selection_highlights(color))
            }
            PluginQuery::FlatLines => {
                PluginResponse::FlatLines(self.preview.flat_lines().to_vec())
            }
            _ => PluginResponse::None,
        }
    }
}

pub(crate) struct MarkdownPluginFactory;

impl PluginFactory for MarkdownPluginFactory {
    fn name(&self) -> &str { "markdown" }
    fn can_handle(&self, path: Option<&Path>) -> bool {
        path.and_then(|p| p.extension())
            .map_or(false, |e| e == "md" || e == "markdown")
    }
    fn create(&self) -> Box<dyn ViewPlugin> {
        Box::new(MarkdownPlugin::new())
    }
}
```

- [ ] **Step 2: 编译**

```bash
cargo build -p edit-plus-app 2>&1 | head -20
```

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/plugins/markdown.rs
git commit -m "refactor(markdown): migrate MarkdownPlugin to ViewPlugin + PluginFactory"
```

---

### Task 6: 适配 DocItem、workspace、所有调用点

**Files:**
- Modify: `crates/app/src/tab.rs`
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/app_scroll.rs`
- Modify: `crates/app/src/app_search.rs`
- Modify: `crates/app/src/dispatch/editor.rs`
- Modify: `crates/app/src/dispatch/mouse.rs`

**Produces:** app 层完全通过 ViewPlugin + PluginRegistry 工作，零 hardcode

- [ ] **Step 1: 重写 DocItem**

```rust
// crates/app/src/tab.rs

use ui::plugin::ViewPlugin;

pub(crate) struct DocItem {
    pub doc: DocumentView,
    pub plugin: Box<dyn ViewPlugin>,
    pub mode_override: Option<ModeOverride>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModeOverride { Reading, Editing }

impl DocItem {
    pub fn new(doc: DocumentView, plugin: Box<dyn ViewPlugin>) -> Self {
        Self { doc, plugin, mode_override: None }
    }

    pub fn file_path(&self) -> Option<&PathBuf> { self.doc.file_path.as_ref() }
    pub fn dirty(&self) -> bool { self.doc.dirty }
    pub fn doc_title(&self) -> String {
        self.doc.file_path.as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("untitled")
            .to_string()
    }
}
```

- [ ] **Step 2: workspace.rs 用 PluginRegistry 路由**

```rust
// 删除 open_file_with_viewport 中的 is_md / is_txt 判断
// 统一走 registry

let plugin = registry.create_for_file(
    Some(path),
    Box::new(EditorPlugin), // fallback
);
self.entries.push(DocItem::new(dv, plugin));
```

- [ ] **Step 3: 全局替换 `preview()/preview_ref()` → `handle_message()/query()`**

```bash
# 找所有引用
git grep "\.preview(" -- "crates/app/src/*.rs"
git grep "\.preview_ref(" -- "crates/app/src/*.rs"
```

逐个替换为 `plugin.handle_message(PluginMessage::...)` 和 `plugin.query(PluginQuery::...)`。

以 `app_scroll.rs` 为例：

```rust
// 旧:
if let Some(preview) = tab.preview() {
    preview.scroll(delta, viewport_h);
}

// 新:
tab.plugin.handle_message(PluginMessage::Scroll { delta, viewport_h }, &mut tab.doc);
```

以 `app_renderer.rs` 为例：

```rust
// 旧:
if let Some(preview) = v.preview_ref() {
    let (dl, _) = preview.render(theme, ...);
}

// 新:
let dl = v.plugin.render(&v.doc, bounds, theme, shaper);
```

- [ ] **Step 4: 编译修复**

```bash
cargo build -p edit-plus-app 2>&1
# 循环修编译错误直至通过
```

- [ ] **Step 5: 验证**

```bash
./scripts/verify.sh
```

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(app): switch to ViewPlugin + PluginRegistry routing"
```

---

### Task 7: 创建 novel crate 骨架

**Files:**
- Create: `crates/novel/Cargo.toml`
- Create: `crates/novel/src/lib.rs`
- Create: `crates/novel/src/chapter.rs`（空）
- Create: `crates/novel/src/merge.rs`（空）
- Create: `crates/novel/src/render.rs`（空）
- Modify: 根 `Cargo.toml`（workspace member）
- Modify: `crates/app/Cargo.toml`（novel optional dep + feature）

**Produces:** 编译通过的 novel crate 空骨架

- [ ] **Step 1: Cargo.toml**

```toml
[package]
name = "edit-plus-novel"
version = "0.1.0"
edition = "2021"

[dependencies]
edit-plus-core = { path = "../core" }
edit-plus-ui = { path = "../ui" }
edit-plus-shaping = { path = "../shaping" }
regex = "1"
```

- [ ] **Step 2: lib.rs**

```rust
//! 小说阅读模式。
//!
//! 实现 `ViewPlugin` + `PluginFactory`，零依赖 app。

pub mod chapter;
pub mod merge;
pub mod render;

use ui::plugin::{ViewPlugin, PluginFactory, PluginMessage, PluginQuery, PluginResponse};
use core::document::{DocView, DocViewMut};
use shaping::Shaper;
use ui::core::geom::Rect;
use ui::core::paint::DrawList;
use ui::theme::Theme;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineStyle { ChapterTitle, Body }

impl LineStyle {
    fn font_size_scale(&self) -> f32 {
        match self { Self::ChapterTitle => 1.5, Self::Body => 1.0 }
    }
    fn font_weight(&self) -> shaping::Weight {
        match self { Self::ChapterTitle => shaping::Weight::BOLD, Self::Body => shaping::Weight::NORMAL }
    }
}

pub struct NovelView {
    chapter_index: chapter::ChapterIndex,
    paragraph_index: merge::ParagraphIndex,
    scroll_y: f32,
    enabled: bool,
    base_font_size: f32,
    line_height_ratio: f32,
}

impl NovelView {
    pub fn new() -> Self {
        Self {
            chapter_index: chapter::ChapterIndex::default(),
            paragraph_index: merge::ParagraphIndex::default(),
            scroll_y: 0.0,
            enabled: true,
            base_font_size: 15.0,
            line_height_ratio: 1.5,
        }
    }

    /// 惰性初始化索引（等待文档可用）。
    fn ensure_index(&mut self, doc: &dyn DocView) {
        if self.chapter_index.entries.is_empty() && doc.line_count() > 0 {
            self.chapter_index = chapter::ChapterIndex::build(doc);
            self.paragraph_index = merge::ParagraphIndex::build(
                doc, &self.chapter_index, self.base_font_size, self.line_height_ratio,
            );
        }
    }

    fn jump_next_chapter(&mut self) {
        let cur = self.chapter_index.index_at_y(self.scroll_y);
        if let Some(entry) = self.chapter_index.entries.get(cur + 1) {
            self.scroll_y = self.paragraph_index.y_for_line(entry.line);
        }
    }

    fn jump_prev_chapter(&mut self) {
        let cur = self.chapter_index.index_at_y(self.scroll_y);
        if cur > 0 {
            self.scroll_y = self.paragraph_index.y_for_line(
                self.chapter_index.entries[cur - 1].line
            );
        } else {
            self.scroll_y = 0.0;
        }
    }
}

impl ViewPlugin for NovelView {
    fn name(&self) -> &str { "novel" }
    fn allows_editing(&self) -> bool { false }
    fn shows_cursor(&self) -> bool { false }
    fn shows_gutter(&self) -> bool { false }

    fn render(&mut self, doc: &dyn DocView, bounds: Rect, theme: &Theme,
              shaper: &mut Shaper) -> DrawList {
        if !self.enabled { return DrawList::new(); }
        self.ensure_index(doc);
        render::render_novel(
            doc, &self.chapter_index, &self.paragraph_index,
            bounds, theme, shaper, self.base_font_size,
            doc.scroll_y(), doc.viewport_height(),
        )
    }

    fn handle_message(&mut self, msg: PluginMessage, doc: &mut dyn DocViewMut) -> bool {
        match msg {
            PluginMessage::Scroll { delta, .. } => {
                self.scroll_y = (self.scroll_y + delta).max(0.0);
                doc.set_scroll_y(self.scroll_y);
                true
            }
            PluginMessage::ScrollToNextChapter => {
                self.jump_next_chapter();
                doc.set_scroll_y(self.scroll_y);
                true
            }
            PluginMessage::ScrollToPrevChapter => {
                self.jump_prev_chapter();
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

- [ ] **Step 3: 空的子模块文件**

```rust
// crates/novel/src/chapter.rs — `//! 章节识别`
// crates/novel/src/merge.rs  — `//! 段落构建`
// crates/novel/src/render.rs — `//! 渲染`
```

- [ ] **Step 4: 根 Cargo.toml workspace members + app/Cargo.toml feature**

```toml
# 根 Cargo.toml members 加 "crates/novel"

# crates/app/Cargo.toml
[dependencies]
edit-plus-novel = { path = "../novel", optional = true }

[features]
novel = ["dep:edit-plus-novel"]
```

- [ ] **Step 5: 编译验证**

```bash
cargo build -p edit-plus-novel
cargo build -p edit-plus-app --features novel
```

- [ ] **Step 6: Commit**

```bash
git add crates/novel/ Cargo.toml crates/app/Cargo.toml
git commit -m "feat(novel): create novel crate skeleton with ViewPlugin impl"
```

---

### Task 8: 实现章节识别

**Files:**
- Modify: `crates/novel/src/chapter.rs`

**Produces:** `ChapterIndex`, `ChapterEntry`, `is_chapter_title()`

- [ ] **Step 1: 写测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_chinese_chapter() {
        assert!(is_chapter_title("第一章 大闹天宫"));
        assert!(is_chapter_title("第三章"));
        assert!(is_chapter_title("第一百二十三章 标题"));
        assert!(is_chapter_title("第3章 新的开始"));
        assert!(is_chapter_title("第12卷 第5章"));
    }

    #[test]
    fn detect_special() {
        assert!(is_chapter_title("序章"));
        assert!(is_chapter_title("楔子"));
        assert!(is_chapter_title("尾声"));
        assert!(is_chapter_title("番外 某年某月"));
    }

    #[test]
    fn detect_english() {
        assert!(is_chapter_title("Chapter 1"));
        assert!(is_chapter_title("Chapter 10 The Beginning"));
    }

    #[test]
    fn reject_body() {
        assert!(!is_chapter_title("话说天下大势，分久必合，合久必分。"));
        assert!(!is_chapter_title(""));
        assert!(!is_chapter_title("第二天一早，张三就起床了。"));
    }

    #[test]
    fn reject_long() {
        let long = format!("第{}章 {}", 1, "长".repeat(130));
        assert!(!is_chapter_title(&long));
    }

    #[test]
    fn reject_punctuation_heavy() {
        assert!(!is_chapter_title("第一章：关于世界，人生，以及一切的一切，最终都归于虚无，我们该如何？"));
    }
}
```

- [ ] **Step 2: 运行确认失败**

```bash
cargo test -p edit-plus-novel
```

- [ ] **Step 3: 实现**

```rust
use std::sync::OnceLock;
use regex::Regex;
use core::document::DocView;

const MAX_TITLE_LENGTH: usize = 120;
const MIN_CHARS_FOR_PUNCT_CHECK: usize = 10;
const MAX_PUNCT_DENSITY: f32 = 0.5;
const MIN_CHARS_FOR_NON_CJK_CHECK: usize = 15;
const MAX_NON_CJK_RATIO: f32 = 0.6;

fn chapter_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(
        r"^(第[一二三四五六七八九十百千万零\d]+[卷章节回]|序章|楔子|尾声|番外[一二三四五六七八九十\d]*|Chapter\s*\d+)"
    ).expect("章节正则语法错误"))
}

pub fn is_chapter_title(text: &str) -> bool {
    if text.is_empty() || text.len() > MAX_TITLE_LENGTH { return false; }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() > MIN_CHARS_FOR_PUNCT_CHECK {
        let punct = chars.iter().filter(|c| c.is_ascii_punctuation() || is_cjk_punctuation(**c)).count();
        if punct as f32 / chars.len() as f32 > MAX_PUNCT_DENSITY { return false; }
    }
    if chars.len() > MIN_CHARS_FOR_NON_CJK_CHECK {
        let non_cjk = chars.iter().filter(|c| !is_cjk_char(**c) && !c.is_ascii_digit() && !c.is_whitespace()).count();
        if non_cjk as f32 / chars.len() as f32 > MAX_NON_CJK_RATIO { return false; }
    }
    chapter_re().is_match(text)
}

fn is_cjk_char(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' | '\u{F900}'..='\u{FAFF}')
}

fn is_cjk_punctuation(c: char) -> bool {
    matches!(c,
        '。' | '，' | '、' | '；' | '：' | '？' | '！'
        | '「' | '」' | '『' | '』' | '（' | '）' | '《' | '》'
        | '…' | '—' | '～' | '・' | '．'
        | '\u{3000}'..='\u{303F}' | '\u{FF00}'..='\u{FFEF}'
    )
}

#[derive(Debug, Clone, Default)]
pub struct ChapterIndex { pub entries: Vec<ChapterEntry> }

#[derive(Debug, Clone)]
pub struct ChapterEntry { pub line: usize, pub title: String }

impl ChapterIndex {
    pub fn build(doc: &dyn DocView) -> Self {
        let mut entries = Vec::new();
        for line in 0..doc.line_count() {
            let text = String::from_utf8_lossy(doc.doc_line_bytes(line));
            let trimmed = text.trim();
            if is_chapter_title(trimmed) {
                entries.push(ChapterEntry { line, title: trimmed.to_string() });
            }
        }
        ChapterIndex { entries }
    }

    pub fn index_at_y(&self, scroll_y: f32) -> usize {
        // 粗略实现：二分查找 — 精确版本需 ParagraphIndex Y 偏移
        // 这里先用行号估算
        0
    }
}
```

- [ ] **Step 4: 测试通过**

```bash
cargo test -p edit-plus-novel
```

- [ ] **Step 5: Commit**

```bash
git add crates/novel/src/chapter.rs
git commit -m "feat(novel): implement chapter title detection"
```

---

### Task 9: 实现段落构建与折行合并

**Files:**
- Modify: `crates/novel/src/merge.rs`

**Produces:** `ParagraphIndex`, `ParagraphEntry`, `merge_paragraph_lines()`

- [ ] **Step 1: 写测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_end_punct() {
        assert!(is_sentence_end('。'));
        assert!(is_sentence_end('？'));
        assert!(!is_sentence_end('的'));
    }

    #[test]
    fn para_start_marker() {
        assert!(is_paragraph_start('\u{3000}'));
        assert!(is_paragraph_start('「'));
        assert!(!is_paragraph_start('话'));
    }
}
```

- [ ] **Step 2: 实现状态机**（同原 plan Task 8 代码，略）

关键函数签名：

```rust
pub fn build(doc: &dyn DocView, chapter_index: &ChapterIndex,
             base_font_size: f32, line_height_ratio: f32) -> ParagraphIndex;

pub fn merge_paragraph_lines(entry: &ParagraphEntry, doc: &dyn DocView) -> String;

pub fn y_for_line(&self, target_line: usize) -> f32;

pub fn visible_range(&self, scroll_y: f32, viewport_h: f32) -> (usize, usize);
```

- [ ] **Step 3: 测试通过并 commit**

---

### Task 10: 实现渲染

**Files:**
- Modify: `crates/novel/src/render.rs`

**Produces:** `render_novel()` → `DrawList`

- [ ] **Step 1: 实现**（同原 plan Task 9 代码）

关键接口：

```rust
pub fn render_novel(
    doc: &dyn DocView,
    chapter_index: &ChapterIndex,
    paragraph_index: &ParagraphIndex,
    bounds: Rect,
    theme: &Theme,
    shaper: &mut Shaper,
    base_font_size: f32,
    scroll_y: f32,
    viewport_h: f32,
) -> DrawList;
```

视口裁剪：二分查找 `visible_range`，仅渲染可见段落。章节标题 1.5x 粗体居中分隔线，正文默认左对齐。

- [ ] **Step 2: Commit**

---

### Task 11: App 集成 novel plugin

**Files:**
- Modify: `crates/ui/src/settings.rs` — `enable_novel_mode`
- Modify: `crates/app/src/app_init.rs` 或等效初始化入口 — registry 注册
- Modify: `crates/app/src/dispatch/editor.rs` — 快捷键

**Produces:** 打开 txt 文件 → NovelView

- [ ] **Step 1: Settings 加配置**

```rust
// crates/ui/src/settings.rs
#[cfg(feature = "novel")]
#[serde(default = "default_enable_novel_mode")]
pub enable_novel_mode: bool,

#[cfg(feature = "novel")]
fn default_enable_novel_mode() -> bool { true }
```

- [ ] **Step 2: 注册 NovelPluginFactory**

```rust
// app 初始化处
let mut registry = PluginRegistry::new();
#[cfg(feature = "markdown")]
registry.register(Box::new(edit_plus_markdown::MarkdownPluginFactory));
#[cfg(feature = "novel")]
if settings.enable_novel_mode {
    registry.register(Box::new(edit_plus_novel::NovelPluginFactory));
}
```

- [ ] **Step 3: 快捷键**

```rust
// Ctrl+Shift+R 切换阅读/编辑
// Cmd+Down → PluginMessage::ScrollToNextChapter
// Cmd+Up → PluginMessage::ScrollToPrevChapter

tab.plugin.handle_message(msg, &mut tab.doc);
```

- [ ] **Step 4: 编译并通过验证**

```bash
cargo build --features novel
cargo test --features novel
./scripts/verify.sh
```

- [ ] **Step 5: Commit**

---

## 验证清单

```bash
cargo build --features novel
cargo build  # 不带 feature 能编译
cargo test --features novel
cargo test
cargo clippy --features novel -- -D warnings
cargo fmt -- --check
./scripts/verify.sh
```
