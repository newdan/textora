# 方案：UI 代码从 crates/app 迁移到 crates/ui

## 背景

`crates/ui` 创建后从未被填充（`lib.rs` 只有一行注释："Populated from stage 5"）。
所有 UI 组件代码目前都在 `crates/app` 里，app 既是 library 又是 binary，职责混杂。

## 架构原则

`crates/ui` 是**纯 UI 组件库**，规则：
- ❌ 不依赖 `DocumentView`
- ❌ 不依赖任何 app 层概念（Workspace、Commands、Events 等）
- ❌ 不做业务逻辑判断
- ✅ 只做渲染：接收纯数据 struct → 输出顶点/布局
- ✅ 可依赖：`core`、`render`、`shaping`、`stdext`
- ✅ 每个 Widget 自己定义输入数据结构，由 app 层负责从 DocumentView 构造

## 现状分析

### 待迁移模块的依赖分析

```
theme.rs          → std only（无内部依赖）                  ✅ 可直接迁
viewport.rs       → snap_tree::DisplayLineEntry              ⚠️ 需抽离 DisplayLineEntry
render_geom.rs    → render::GlyphVertex（外部 crate）         ✅ 可直接迁
layout.rs         → shaping + render_geom::AdvanceCacheEntry  ✅ 可直接迁（render_geom 迁了就行）
scrollbar.rs      → render::GlyphVertex + theme::Theme        ✅ 可直接迁（theme 迁了就行）
gutter.rs         → render + shaping + ATLAS_SIZE + settings  ⚠️ 需提取输入 struct
search_bar.rs     → render + search_state + render_pipeline   ⚠️ 需提取输入 struct
tab_bar.rs        → render + DocumentView + settings + theme  ⚠️ 需提取 TabInfo struct
status_bar.rs     → DocumentView                              ⚠️ 需提取 StatusInfo struct
decorations.rs    → render + DocumentView + render_geom       ⚠️ 需提取输入 struct
settings.rs       → 纯配置数据                                ✅ 可直接迁
```

### 关于 viewport.rs 的问题

`viewport.rs` 依赖 `snap_tree::DisplayLineEntry`。`snap_tree` 是文档模型层的东西，不应该进 ui。
解决方案：viewport 的公开 API 用基本类型（`usize`、`f32`），内部用泛型，或者把 `DisplayLineEntry` 的关键字段提取。

## 方案设计

### 阶段 1：迁移纯数据和原语（低风险）

**范围**：`theme.rs`、`render_geom.rs`、`settings.rs`

| 文件 | 行数 | 迁移难度 | 说明 |
|------|------|----------|------|
| `theme.rs` | 180 | 无 | 零外部依赖 |
| `render_geom.rs` | 110 | 无 | 只依赖外部 `render` crate |
| `settings.rs` | 378 | 无 | 纯配置结构体，app 可通过 `ui::Settings` 引用 |

**具体工作**：
1. 移动 3 个文件到 `crates/ui/src/`
2. 在 `ui/src/lib.rs` 中 `pub mod` 导出
3. `ui/Cargo.toml` 添加 `render`、`serde`（settings 需要）依赖
4. app 中 `use ui::theme`、`use ui::render_geom`、`use ui::Settings` 替换原引用
5. `cargo check` 验证

### 阶段 2：迁移无模型依赖的 Widget

**范围**：`layout.rs`、`scrollbar.rs`

| 文件 | 行数 | 迁移难度 | 说明 |
|------|------|----------|------|
| `layout.rs` | 355 | 无 | 纯布局数学，依赖 `shaping` + `render_geom`（已迁） |
| `scrollbar.rs` | 647 | 无 | 只依赖 `render` + `theme`（已迁） |

### 阶段 3：模型解耦 + 迁移复杂 Widget（核心阶段）

**核心思路**：每个 Widget 定义自己的**纯数据输入 struct**，完全不引用 `DocumentView`。
由 app 层负责"解构 DocumentView → 填 Widget 输入 struct → 调 Widget render"。

#### 3a. 定义输入数据结构

在 `ui` 中为每个 Widget 定义输入 struct：

```rust
// --- ui/src/tab_bar.rs ---

/// 标签栏需要的每个标签的数据
pub struct TabInfo {
    pub title: String,
    pub path: Option<PathBuf>,
    pub is_dirty: bool,
    pub language: String,
}

/// 标签栏渲染输入
pub struct TabBarInput<'a> {
    pub tabs: &'a [TabInfo],
    pub active_index: usize,
    pub width_px: f32,
    pub scale_factor: f64,
    pub settings: &'a Settings,
    pub theme: &'a Theme,
}

pub fn render(input: &TabBarInput) -> Vec<GlyphVertex> { ... }
pub fn hit_test(input: &TabBarInput, x: f32, y: f32) -> TabBarHit { ... }
pub fn tab_bar_height(settings: &Settings) -> f32 { ... }
```

```rust
// --- ui/src/status_bar.rs ---

pub struct StatusBarInput<'a> {
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub total_lines: usize,
    pub file_encoding: String,
    pub line_endings: String,
    pub language: String,
    pub width_px: f32,
    pub scale_factor: f64,
    pub settings: &'a Settings,
    pub theme: &'a Theme,
}

pub fn render(input: &StatusBarInput) -> Vec<GlyphVertex> { ... }
```

```rust
// --- ui/src/gutter.rs ---

pub struct GutterInput<'a> {
    pub visible_lines: &'a [GutterLineInfo],
    pub settings: &'a Settings,
    pub theme: &'a Theme,
}

pub struct GutterLineInfo {
    pub doc_line_idx: usize,   // 1-based display line number
    pub y_offset_px: f32,      // vertical pixel position
}

pub fn render(input: &GutterInput) -> Vec<GlyphVertex> { ... }
```

```rust
// --- ui/src/search_bar.rs ---

pub struct SearchBarInput<'a> {
    pub query: &'a str,
    pub match_count: usize,
    pub current_match: usize,
    pub is_visible: bool,
    pub width_px: f32,
    pub scale_factor: f64,
    pub settings: &'a Settings,
    pub theme: &'a Theme,
}

pub fn render(input: &SearchBarInput) -> Vec<GlyphVertex> { ... }
```

```rust
// --- ui/src/decorations.rs ---

pub struct DecorationInput<'a> {
    pub selections: &'a [SelectionRange],
    pub highlight_spans: &'a [HighlightSpan],
    pub settings: &'a Settings,
    pub theme: &'a Theme,
}

pub struct SelectionRange {
    pub start_byte: usize,
    pub end_byte: usize,
}

pub struct HighlightSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub kind: HighlightKind,
}

pub fn render(input: &DecorationInput) -> Vec<GlyphVertex> { ... }
```

#### 3b. app 层适配

在 `app` 中，不再是 `tab_bar::render(&doc_view, ...)`，而是：

```rust
// app 层负责构造输入
let tab_infos: Vec<ui::TabInfo> = workspace.tabs.iter().map(|tab| {
    ui::TabInfo {
        title: tab.doc_view.title().to_string(),
        path: tab.doc_view.path().cloned(),
        is_dirty: tab.doc_view.is_dirty(),
        language: tab.doc_view.language().to_string(),
    }
}).collect();

let input = ui::TabBarInput {
    tabs: &tab_infos,
    active_index: workspace.active_index,
    width_px: window_width,
    scale_factor,
    settings: &self.settings,
    theme: &self.current_theme,
};
let vertices = ui::tab_bar::render(&input);
```

#### 3c. viewport.rs 处理

`viewport.rs` 依赖 `snap_tree::DisplayLineEntry` —— 这是文档内部的类型，不应进 ui。
方案：viewport 本身是纯几何类型（`ViewportRect`、`ScrollOffset` 等），只需在 ui 中定义简单的几何类型，去掉对 `DisplayLineEntry` 的依赖。

实际检查：viewport 里的 `DisplayLineEntry` 引用是做什么用的？
— 如果只是类型传递，可以改成泛型或用 trait bound
— 如果是计算逻辑，应该留在 app 层

#### 3d. 迁移清单

| 文件 | 行数 | 需要定义的输入 struct |
|------|------|----------------------|
| `gutter.rs` | 145 | `GutterInput`、`GutterLineInfo` |
| `search_bar.rs` | 147 | `SearchBarInput` |
| `tab_bar.rs` | 1631 | `TabBarInput`、`TabInfo` |
| `status_bar.rs` | 64 | `StatusBarInput` |
| `decorations.rs` | 206 | `DecorationInput`、`SelectionRange`、`HighlightSpan` |
| `viewport.rs` | 845 | 去 `DisplayLineEntry` 依赖 |
| `layout.rs` | 355 | 已无问题（阶段 2） |

### 阶段 4：收尾清理

- 移除 `app` 中的旧文件
- `ui/src/lib.rs` 统一导出
- 运行测试
- 更新 AGENTS.md

## 最终依赖关系

```
┌─────────────────────────────────────────┐
│  crates/ui (纯 UI 组件库)                 │
│  - theme, settings, viewport             │
│  - render_geom, layout                   │
│  - tab_bar, status_bar, search_bar       │
│  - scrollbar, gutter, decorations        │
│                                          │
│  依赖: core, render, shaping, stdext     │
│  不依赖: DocumentView, Workspace, App    │
└──────────────┬──────────────────────────┘
               │ use ui::*
┌──────────────▼──────────────────────────┐
│  crates/app (应用层)                      │
│  - app.rs, events.rs, commands.rs        │
│  - workspace.rs, document_view.rs        │
│  - render_pipeline, gpu, input, mouse    │
│  - cursor_motion, display_line_map       │
│                                          │
│  职责: 从 DocumentView 提取数据           │
│       构造 Widget 输入 → 调 ui::render()  │
└─────────────────────────────────────────┘
```

## 工作量估算

| 阶段 | 文件改动 | 新增代码 | 难度 |
|------|---------|---------|------|
| 阶段 1 | ~6 个 | 少 | 低 |
| 阶段 2 | ~6 个 | 少 | 低 |
| 阶段 3 | ~20 个 | 中等（输入 struct 定义 + app 适配） | 中 |
| 阶段 4 | ~8 个 | 无 | 低 |

## 建议执行策略

1. 先做阶段 1+2，让 `ui` 有实质内容，验证架构可行
2. 阶段 3 按 Widget 逐个迁移（tab_bar 先迁，验证模式可行后再迁其他的）
3. 每个 Widget 迁移完立即 `cargo check` 确保不累积问题
