# MdView 技术方案：Markdown 预览视图独立化

## 问题

Markdown 预览状态（`is_markdown_preview` + `MarkdownPreview`）全局存放在 `UiShell` 上：
- 切换 tab 时预览状态不跟随文档
- 非 .md 文件也会残留预览态
- 预览和编辑的职责混在同一个渲染路径里

## 目标

预览状态 per-tab，随 `DocumentView` 生命期走。

---

## 架构设计

OOD 类比：`DocumentView` 是基类，`MdView` 是派生类（`DocumentView` + 预览能力）。Rust 用组合 + enum 实现：

```rust
// crates/app/src/view.rs（新文件）

/// 预览专有状态（缓存 + 滚动）
pub(crate) struct MdPreviewState {
    pub scroll_y: f32,
    pub content_height: f32,
    cached_layout: Option<LaidOutDoc>,
    cached_style_hash: u64,
    cached_source_hash: u64,
    cached_generation: u32,
    dirty: bool,
}

/// DocumentView + 预览能力
pub(crate) struct MdView {
    pub doc: DocumentView,
    pub preview: MdPreviewState,
}

/// View 替代 DocumentView 作为 Workspace 的存储单元
pub(crate) enum View {
    Editor(DocumentView),
    Markdown(MdView),
}

impl View {
    pub fn doc(&self) -> &DocumentView;
    pub fn doc_mut(&mut self) -> &mut DocumentView;
    pub fn file_path(&self) -> Option<&PathBuf>;
    pub fn is_dirty(&self) -> bool;
    pub fn is_markdown(&self) -> bool;          // matches!(self, View::Markdown(_))

    pub fn into_editor(self) -> Self;            // Markdown → Editor（保留 doc）
    pub fn into_markdown(self) -> Self;          // Editor → Markdown（需 .md 检查）
}
```

### 缓存策略

三级缓存，按开销分层，避免 O(n) 全文提取：

1. **`cached_generation`** — gap buffer 代数。O(1) 判断 buffer 可能被编辑过，未变则跳过全文提取
2. **`cached_source_hash`** — 文本 hash。O(n) 精确判断内容是否真变了（generation 可能回绕或 undo 回到相同内容）
3. **`cached_style_hash`** — 主题参数 hash。主题变了同一文本也需重建 layout

三层都通过才复用 `cached_layout`。`content_height` 从 layout 结果赋值但独立存储（高频读取，避免每次从 Option 解包）。

### Workspace 变更

```rust
// 改前
pub(crate) struct Workspace {
    pub(crate) doc_views: Vec<DocumentView>,
    pub(crate) preview_index: Option<usize>,     // 删除：quick-open 预览标签
    ...
}

// 改后
pub(crate) struct Workspace {
    pub(crate) views: Vec<View>,
    ...
    // active_doc() / active_doc_mut() 保留为便捷方法，内部解包 View
}
```

### UiShell 变更

删除三个字段/方法：
- `is_markdown_preview: bool`
- `markdown_preview: Option<MarkdownPreview>`
- `toggle_markdown_preview()`

### 渲染/滚动分支

从全局 bool 改为 View 模式匹配：

```rust
// app_renderer.rs — 渲染分支
match workspace.active_view() {
    Some(View::Markdown(mv)) => { /* 预览渲染 */ }
    _ => { /* 编辑渲染 */ }
}

// app_scroll.rs — 滚动分支
match workspace.active_view_mut() {
    Some(View::Markdown(mv)) => { mv.preview.scroll(dy, viewport_h); }
    _ => { /* 编辑器滚动 */ }
}
```

---

## 涉及文件

| 文件 | 改动 |
|------|------|
| `view.rs`（新） | `View` enum + `MdView` + `MdPreviewState` + 委托方法 |
| `workspace.rs` | `Vec<DocumentView>` → `Vec<View>`，删 `preview_index` |
| `ui_shell.rs` | 删预览相关字段和方法 |
| `app_renderer.rs` | match `View` 分支渲染 |
| `app_scroll.rs` | match `View` 分支滚动 |
| `app_dispatch.rs` | `ToggleMarkdownPreview` 改为 View 转换 |
| `app_window.rs` | 适配 View 接口 |
| `events.rs` | 适配 View 接口 |
| `lib.rs` | 加 `pub mod view;` |
| `md_preview.rs` | 删除（逻辑移入 `view.rs` 的 `MdPreviewState`/`MdView`） |

---

## 分阶段实施

### Phase 1: 类型定义 + Workspace 适配
- 创建 `crates/app/src/view.rs`
- 定义 `View` enum + `MdView` + `MdPreviewState` + 委托方法
- `lib.rs` 加 `pub mod view;`
- Workspace `doc_views` → `views`，保留 `active_doc()` / `active_doc_mut()`
- 适配所有 `doc_views` 访问点
- **验证**：`cargo check -p edit-plus-app`

### Phase 2a: MdPreviewState 渲染/滚动方法
- `MdPreviewState` 实现 `render()` / `scroll()` / `set_source()`（从 worktree 原型搬）
- **验证**：`cargo check -p edit-plus-app`

### Phase 2b: renderer/scroll 切换到 View 匹配
- `app_renderer.rs`：match `View` 决定渲染路径
- `app_scroll.rs`：match `View` 决定滚动路径
- 预览态键盘：只响应导航（PageDown/PageUp/方向键），编辑输入忽略
- **验证**：`cargo check -p edit-plus-app`

### Phase 2c: UiShell 清理
- 删除 `UiShell` 中 `is_markdown_preview` / `markdown_preview` / `toggle_markdown_preview()`
- **验证**：`cargo check -p edit-plus-app`

### Phase 3: Toggle 命令适配
- `ToggleMarkdownPreview`：
  - 检查 active view 文件扩展名是 .md，否则忽略
  - `View::Editor(dv)` → `View::Markdown(MdView::from(dv))`
  - `View::Markdown(mv)` → `View::Editor(mv.into_doc())`
- 切换 tab：View 是 per-tab 的，`switch_to()` 无需额外处理
- **验证**：`cargo test --lib` + 手动测试

### Phase 4: 清理 + 全量验证
- 删除 `md_preview.rs`
- `cargo check -p edit-plus-app`（零 warning）
- `cargo test --lib`

---

## 边界情况

| 场景 | 处理 |
|------|------|
| 非 .md 文件执行 ToggleMarkdownPreview | 忽略（View 保持 Editor） |
| Markdown 预览态下关闭 tab | `View` drop 自动清理 |
| 切换 tab 后预览滚动位置 | 保留（per-MdView 的 `scroll_y`） |
| 主题变化 | `cached_style_hash` 检测，自动重建 |
| 预览态下导航键（PageDown/↑↓） | 正常响应滚动 |
| 预览态下编辑输入（字符键等） | 忽略，不退出预览 |
| 预览态下 undo/redo | 忽略（buffer 变化通过 generation 检测感知，但不切回编辑） |

---

## 状态

待实施。
