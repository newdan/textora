# MarkdownView 改造方案（原 MarkdownPreview）

## 1. 现状诊断

### 1.1 核心问题：插件实现不对称

```
NovelView (crates/novel)     → 直接 impl ViewPlugin  ← 正确
MarkdownView (crates/markdown) → 不 impl ViewPlugin   ← 不一致
  └── MarkdownPlugin (crates/app) → impl ViewPlugin（200行胶水代码）
```

导致三个后果：
- `markdown` crate 无法独立作为 `ViewPlugin` 使用
- `app` 必须维护 `MarkdownPlugin` 适配器，纯粹是委托样板代码
- `app/src/lib.rs:104` 通过 `pub(crate) use edit_plus_markdown::preview as md_preview` re-export，让 `app` 层直接构造 `crate::md_preview::PreviewPos` 等 markdown 内部类型——跨层紧耦合

### 1.2 次要问题：方法过长

`MarkdownView`（现 `preview.rs`，954行）中有几个方法超过 50 行：

| 方法 | 行数 | 说明 |
|------|------|------|
| `render()` | ~130行 | 缓存检测 + parse/build/layout + 精度补偿 + 渲染调度 |
| `word_at_pos()` | ~90行 | 内嵌了 `char_class` 枚举和分类逻辑 |
| `render_line_with_offset()` (render.rs) | ~350行 | 这是另一文件，但同样是巨型函数 |

### 1.3 代码异味：RefCell 绕过借用检查

```rust
// 原 preview.rs:162-165 — 为了在 &self 方法中更新搜索缓存
cached_search_query: RefCell<String>,
cached_search_case_sensitive: Cell<bool>,
cached_search_generation: Cell<u32>,
cached_search_rects: RefCell<Vec<Rect>>,
```

四个带内部可变性的字段，本质上是一个独立的小状态机被迫塞进 `&self`。

---

## 2. 目标架构

```
crates/markdown/src/
  view.rs        ← MarkdownView（impl ViewPlugin）+ 渲染缓存 + 滚动 + TOC
  selection.rs   ← ViewPos + 选区相关方法（hit_test/word_at_pos/line_range/highlights）
  search.rs      ← SearchState（消除 RefCell）
```

```
crates/app/src/plugins/markdown.rs  → 删除
crates/app/src/lib.rs              → 删除 md_preview re-export
```

### 2.1 为什么只拆 2 个子模块

- **`selection.rs`** 有必要：`ViewPos` 是一个独立的数据类型，hit-test/word/line 是自成体系的纯函数族（~340行），与渲染管线无关
- **`search.rs`** 有必要：4 个 `Cell`/`RefCell` 字段说明它本来就是被强行塞进 `&self` 的独立状态机。拆出去给 `&mut self`，RefCell 自然消失
- **其他不拆**：滚动（2 字段 + 3 个小方法）、TOC（依赖 lazy layout 的遍历，方法 <20行）、渲染缓存（与 render 紧密耦合）——拆出去只是增加文件数，不增加清晰度

---

## 3. 拆分详设

### 3.1 `selection.rs` — 从 view.rs 移出

移入内容：
- `ViewPos` struct（原 `PreviewPos`）
- `char_at_x()` / `char_x()` 私有辅助函数
- `preview_hit_test()` → 重命名为 `hit_test()`
- `word_at_pos()` → 拆分为 `word_at_pos()` + 独立函数 `char_class()`
- `line_range_at_pos()`
- `preview_selection_range()` → `selection_range()`
- `preview_selected_text()` → `selected_text()`
- `preview_select_all()` → `select_all()`
- `clear_preview_selection()` → `clear()`
- `has_preview_selection()` → `has_selection()`
- `selection_highlights()` → `highlights()`

`MarkdownView` 上保留 `sel_anchor` / `sel_cursor` 字段，但所有逻辑委托给 `SelectionState` 的纯函数。

### 3.2 `search.rs` — 消除 RefCell

```rust
pub(crate) struct SearchState {
    query: String,
    case_sensitive: bool,
    generation: u32,
    rects: Vec<Rect>,
}

impl SearchState {
    pub fn new() -> Self;

    /// 增量更新搜索矩形缓存（需要 &mut self，不再需要 RefCell）。
    pub fn update(&mut self, query: &str, case_sensitive: bool, generation: u32, flat_lines: &[FlatLine]);

    /// 生成搜索高亮 DrawList。
    pub fn highlights(&self, scroll_y: f32, viewport_h: f32,
                       offset_x: f32, offset_y: f32,
                       active_idx: usize, match_color: [f32; 4],
                       inactive_color: [f32; 4]) -> DrawList;

    /// 滚动到第 N 个匹配项。
    pub fn scroll_to(&self, active_idx: usize, viewport_h: f32) -> Option<f32>;
}
```

### 3.3 `view.rs` — 精简后的 MarkdownView

保留内容：
- 缓存状态字段（source、hashes、dirty、lazy layout）
- 渲染缓存字段（cached_dl、cached_vertices）
- 滚动字段（scroll_y、content_height、pending_heading_jump）
- TOC 字段（headings）
- `sel_anchor` / `sel_cursor`（选区锚点）
- `SelectionState` / `SearchState` 作为组合字段

**方法拆分**：`render()` 从 ~130 行拆为：

```rust
fn render(&mut self, ...) -> (DrawList, bool) {
    // 1. 检测是否需要重建布局
    if self.needs_rebuild(style_hash, viewport_w) {
        self.rebuild_layout(&style, viewport_w, shaper, &highlighter);
    }

    // 2. 尝试返回缓存
    if let Some(dl) = self.render_cache.hit(self.scroll_y, viewport_w, viewport_h) {
        return (dl, false);
    }

    // 3. 精度补偿 + 渲染
    self.precision_pass_and_render(&style, viewport_w, viewport_h, offset_x, offset_y, shaper)
}
```

`word_at_pos()` 内嵌的 `char_class` 提升为模块级独立函数。

### 3.4 impl ViewPlugin

`MarkdownView` 直接实现 `ViewPlugin`，不再需要 `MarkdownPlugin` 适配：

```rust
impl ViewPlugin for MarkdownView {
    fn name(&self) -> &str { "markdown_view" }
    fn allows_editing(&self) -> bool { false }
    fn shows_cursor(&self) -> bool { false }
    fn shows_gutter(&self) -> bool { false }

    fn render(&mut self, _doc: &dyn DocView, bounds: Rect, theme: &Theme, shaper: &mut Shaper) -> DrawList {
        // 从 Theme + bounds 推导 render settings
        // 调用内部 render 管线
    }

    fn handle_message(&mut self, msg: PluginMessage, _doc: &mut dyn DocViewMut) -> bool {
        // 直接分发到 self 方法（不再经过 MarkdownPlugin 转发）
    }

    fn query(&self, q: PluginQuery, _doc: &dyn DocView) -> PluginResponse {
        // 直接分发
    }
}
```

---

## 4. 拆除 app 层适配

### 4.1 删除 `MarkdownPlugin`

`crates/app/src/plugins/markdown.rs` 整文件删除。`MarkdownPluginFactory` 移到 `markdown` crate，`create()` 返回 `Box::new(MarkdownView::new())`。

### 4.2 删除 `md_preview` re-export

`app/src/lib.rs` 中删除：
```rust
pub(crate) use edit_plus_markdown::preview as md_preview;
```

`app_renderer.rs` 中对 `crate::md_preview::*` 的直接引用改为通过 `ViewPlugin` trait 方法获取。

---

## 5. 迁移步骤

### 步骤 1：提取 SearchState（不改 API）

- 在 `search.rs` 中定义 `SearchState`
- `MarkdownView` 将 4 个 `Cell`/`RefCell` 替换为一个 `SearchState` 字段
- `update_search_rects_if_needed` → `SearchState::update`
- 编译 + 测试

### 步骤 2：提取 SelectionState

- 在 `selection.rs` 中定义 `SelectionState` + 所有纯函数
- `MarkdownView` 委托选区方法
- 保持 `pub` API 不变
- 编译 + 测试

### 步骤 3：实现 ViewPlugin

- `MarkdownView` 直接 `impl ViewPlugin`
- `MarkdownPluginFactory` 移到 `markdown` crate
- 编译 + 测试

### 步骤 4：清理 app 层

- 删除 `app/src/plugins/markdown.rs`
- 删除 `md_preview` re-export
- 修正 `app_renderer.rs` 引用
- 编译 + 测试

### 步骤 5：验证

- `cargo test -p edit-plus-markdown`
- `cargo test -p edit-plus-app`
- `./scripts/verify.sh`
- 手动测试：打开 .md 文件，预览渲染、滚动、选区、搜索、TOC 跳转

---

## 6. 验收标准

- [ ] `MarkdownView` 直接 `impl ViewPlugin`，`app` 层无适配器
- [ ] `app` 层不直接引用 `markdown::preview` 的任何类型
- [ ] `SearchState` 无 `Cell`/`RefCell`
- [ ] `render()` 方法 ≤ 50 行
- [ ] `word_at_pos()` 方法 ≤ 50 行
- [ ] 全部现有测试通过
- [ ] 手动测试：markdown 预览所有功能正常
