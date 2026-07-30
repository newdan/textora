# 消除 legacy_md_preview — 详细实现方案

> **目标：** 用 `PreviewPlugin` trait 替换所有 `legacy_md_preview()` / `legacy_md_preview_mut()` downcast，然后移除 `ContentPlugin::as_any()`。

## 现状分析

### 调用点统计（22 处）

| 文件 | 调用数 | 访问模式 |
|------|--------|---------|
| `dispatch/editor.rs` | 8 | 7× `preview_mut()` + 1× `preview()`；大量直接访问 `sel_cursor`/`sel_anchor` 字段 |
| `app_renderer.rs` | 9 | 混合 `preview()` 和 `preview_mut()`；访问 `scroll_y`/`content_height` 字段 |
| `dispatch/mouse.rs` | 2 | `preview_mut()`；`preview_hit_test()` |
| `dispatch/viewport.rs` | 2 | `preview_mut()`；`scroll()` |
| `app_scroll.rs` | 1 | `preview_mut()`；`scroll()` |
| `app_search.rs` | 1 | `preview_mut()`；`scroll_to_search_match()` |

### 直接字段访问（必须转为 getter/setter）

| 字段 | 访问次数 | 语义 |
|------|---------|------|
| `mv.sel_cursor` | 25 | 选区光标位置 |
| `mv.sel_anchor` | 19 | 选区锚点位置 |
| `mv.scroll_y` | 2+ | 垂直滚动偏移 |
| `mv.content_height` | 1 | 内容总高度 |

### 方法调用（直接映射到 trait）

`scroll()`, `scroll_to_heading()`, `scroll_to_search_match()`, `headings()`, `current_heading_index()`, `needs_source_update()`, `set_source()`, `render()`, `cache_vertices()`, `get_cached_vertices()`, `anchor()`, `restore_anchor()`, `preview_hit_test()`, `preview_selection_range()`, `preview_selected_text()`, `selection_highlights()`, `has_preview_selection()`, `clear_preview_selection()`, `preview_select_all()`, `word_at_pos()`, `line_range_at_pos()`, `search_highlights()`, `flat_lines()`

---

## 设计方案

### 核心思路

创建 `PreviewPlugin` trait，包含预览插件的所有能力方法。`MarkdownPlugin` 实现此 trait。`ContentPlugin` 新增 `preview()` / `preview_ref()` 方法返回 `Option<&(mut) dyn PreviewPlugin>`。所有调用点从 `legacy_md_preview()` 迁移到 `preview()` / `preview_ref()`。

### 新增文件

#### `crates/app/src/preview_plugin.rs`

```rust
pub(crate) trait PreviewPlugin {
    // ── 滚动 ──
    fn scroll_y(&self) -> f32;
    fn content_height(&self) -> f32;
    fn scroll(&mut self, delta: f32, viewport_h: f32) -> bool;
    fn scroll_to_heading(&mut self, index: usize);
    fn scroll_to_search_match(&mut self, match_idx: usize, query: &str, match_case: bool, use_regex: bool);

    // ── TOC ──
    fn headings(&self) -> &[HeadingEntry];
    fn current_heading_index(&self, scroll_y: f32) -> Option<usize>;

    // ── 源码同步 ──
    fn needs_source_update(&self, generation: u32) -> bool;
    fn set_source(&mut self, text: String, generation: u32);

    // ── 渲染 ──
    fn render(&mut self, theme: &Theme, viewport_w: f32, viewport_h: f32,
              offset_x: f32, offset_y: f32, settings: MarkdownRenderSettings,
              shaper: Option<&mut Shaper>) -> (DrawList, bool);
    fn cache_vertices(&mut self, verts: Vec<GlyphVertex>);
    fn get_cached_vertices(&self) -> Option<&Vec<GlyphVertex>>;
    fn anchor(&self) -> Option<BlockAnchor>;
    fn restore_anchor(&mut self, anchor: &BlockAnchor);

    // ── 选区 ──
    fn sel_cursor(&self) -> Option<PreviewPos>;
    fn set_sel_cursor(&mut self, pos: Option<PreviewPos>);
    fn sel_anchor(&self) -> Option<PreviewPos>;
    fn set_sel_anchor(&mut self, pos: Option<PreviewPos>);
    fn preview_hit_test(&self, px: f32, py: f32, offset_x: f32, offset_y: f32) -> Option<PreviewPos>;
    fn preview_selection_range(&self) -> Option<(PreviewPos, PreviewPos)>;
    fn preview_selected_text(&self) -> Option<String>;
    fn selection_highlights(&self, sel_color: [f32; 4]) -> DrawList;
    fn has_preview_selection(&self) -> bool;
    fn clear_preview_selection(&mut self);
    fn preview_select_all(&mut self);
    fn word_at_pos(&self, pos: PreviewPos) -> (PreviewPos, PreviewPos);
    fn line_range_at_pos(&self, pos: PreviewPos) -> (PreviewPos, PreviewPos);

    // ── 搜索 ──
    fn search_highlights(&self, query: &str, case_sensitive: bool, use_regex: bool,
                         active_match_idx: usize, match_color: [f32; 4],
                         inactive_color: [f32; 4]) -> DrawList;

    // ── 数据 ──
    fn flat_lines(&self) -> &[FlatLine];
}
```

### 修改文件

#### 1. `crates/app/src/plugin.rs` — ContentPlugin trait

```diff
+ use crate::preview_plugin::PreviewPlugin;

  pub(crate) trait ContentPlugin {
      // ... 现有方法 ...

+     /// 获取预览插件接口（可变）。非预览插件返回 None。
+     fn preview(&mut self) -> Option<&mut dyn PreviewPlugin> { None }
+
+     /// 获取预览插件接口（只读）。非预览插件返回 None。
+     fn preview_ref(&self) -> Option<&dyn PreviewPlugin> { None }
-
-     fn as_any(&self) -> &dyn Any;
-     fn as_any_mut(&mut self) -> &mut dyn Any;
  }
```

#### 2. `crates/app/src/plugins/markdown.rs` — MarkdownPlugin 实现

```diff
+ use crate::preview_plugin::PreviewPlugin;

  impl ContentPlugin for MarkdownPlugin {
-     fn as_any(&self) -> &dyn Any { self }
-     fn as_any_mut(&mut self) -> &mut dyn Any { self }
+     fn preview(&mut self) -> Option<&mut dyn PreviewPlugin> { Some(self) }
+     fn preview_ref(&self) -> Option<&dyn PreviewPlugin> { Some(self) }
  }

+ impl PreviewPlugin for MarkdownPlugin {
+     fn scroll_y(&self) -> f32 { self.preview.scroll_y }
+     fn content_height(&self) -> f32 { self.preview.content_height }
+     fn sel_cursor(&self) -> Option<PreviewPos> { self.preview.sel_cursor.clone() }
+     fn set_sel_cursor(&mut self, pos: Option<PreviewPos>) { self.preview.sel_cursor = pos; }
+     fn sel_anchor(&self) -> Option<PreviewPos> { self.preview.sel_anchor.clone() }
+     fn set_sel_anchor(&mut self, pos: Option<PreviewPos>) { self.preview.sel_anchor = pos; }
+     // ... 委托 self.preview 的所有方法 ...
+ }
```

#### 3. `crates/app/src/tab.rs` — 移除 legacy 方法

```diff
- pub(crate) fn legacy_md_preview(&self) -> Option<&MarkdownPreview> { ... }
- pub(crate) fn legacy_md_preview_mut(&mut self) -> Option<&mut MarkdownPreview> { ... }
```

#### 4. 迁移调用点（6 个文件）

**迁移模式：**

```rust
// 旧代码（只读）:
if let Some(mv) = tab.legacy_md_preview() { mv.scroll_y }

// 新代码:
if let Some(p) = tab.plugin.preview_ref() { p.scroll_y() }

// 旧代码（可变）:
if let Some(mv) = tab.legacy_md_preview_mut() { mv.scroll(10.0, vh) }

// 新代码:
if let Some(p) = tab.plugin.preview() { p.scroll(10.0, vh) }

// 旧代码（直接字段赋值）:
mv.sel_cursor = Some(new_pos);
mv.sel_anchor = mv.sel_cursor;

// 新代码:
p.set_sel_cursor(Some(new_pos));
p.set_sel_anchor(p.sel_cursor());
```

#### 5. `crates/app/src/lib.rs` — 添加模块声明

```diff
+ mod preview_plugin;
```

---

## 任务拆分

### Task A: 创建 PreviewPlugin trait + MarkdownPlugin 实现
- 新建 `preview_plugin.rs`
- 修改 `plugin.rs`（加 `preview()`/`preview_ref()`，删 `as_any`）
- 修改 `markdown.rs`（实现 PreviewPlugin，更新 ContentPlugin impl）
- 修改 `tab.rs`（删 `legacy_md_preview*`，加 `preview()` 便捷方法）
- 修改 `lib.rs`（加 `mod preview_plugin`）
- **此时编译会失败**（调用点还引用旧方法）— 预期内

### Task B: 迁移 dispatch/editor.rs（最大文件，8 调用点）
- 替换所有 `legacy_md_preview*()` → `preview()`/`preview_ref()`
- 替换所有 `mv.sel_cursor`/`mv.sel_anchor` 直接访问 → getter/setter
- 逐个 EditCommand 匹配块修改

### Task C: 迁移 app_renderer.rs（9 调用点）
- 替换所有 `legacy_md_preview*()` → `preview()`/`preview_ref()`
- 替换 `mv.scroll_y`/`mv.content_height` → 方法调用
- 注意 `render()` 签名差异（需要传 `offset_x`, `offset_y`, `settings`）

### Task D: 迁移剩余 4 个文件
- `dispatch/mouse.rs`（2 处）
- `dispatch/viewport.rs`（2 处）
- `app_scroll.rs`（1 处）
- `app_search.rs`（1 处）

### Task E: 验证 + 提交
- `cargo check`
- `cargo test`
- `cargo fmt`
- `./scripts/verify.sh`
- 提交

---

## 风险评估

| 风险 | 等级 | 缓解措施 |
|------|------|---------|
| `render()` 签名不匹配 | 中 | PreviewPlugin trait 的 `render()` 保持与 `MarkdownPreview::render()` 相同签名 |
| `PreviewPos` 需要 `Clone` | 低 | 已有 `pub(crate)` 字段，确认是否 derive Clone |
| 编译错误过多 | 中 | Task A 完成后编译必挂，Task B-D 逐步修复 |
| `preview_ref()` 需要 `&self` 但 ContentPlugin 是 `&dyn` | 低 | trait 方法默认返回 None，MarkdownPlugin override |

---

## 文件变更汇总

| 文件 | 变更类型 |
|------|---------|
| `preview_plugin.rs` | **新建** — PreviewPlugin trait 定义 |
| `plugin.rs` | 修改 — 加 `preview()`/`preview_ref()`，删 `as_any` |
| `plugins/markdown.rs` | 修改 — 实现 PreviewPlugin |
| `tab.rs` | 修改 — 删 legacy 方法 |
| `lib.rs` | 修改 — 加 `mod preview_plugin` |
| `dispatch/editor.rs` | 修改 — 8 处迁移 |
| `app_renderer.rs` | 修改 — 9 处迁移 |
| `dispatch/mouse.rs` | 修改 — 2 处迁移 |
| `dispatch/viewport.rs` | 修改 — 2 处迁移 |
| `app_scroll.rs` | 修改 — 1 处迁移 |
| `app_search.rs` | 修改 — 1 处迁移 |
