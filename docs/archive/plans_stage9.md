# 阶段 9 执行方案：多 buffer + Tab UI

## 概述

将 `App` 从单 `DocumentView` 扩展为多文档编辑器，加 Tab Bar UI +
macOS 原生文件对话框。同时完成 macOS 深色/浅色主题跟随。

## 子任务拆分

---

### 9.1 主题系统（可配置，预留语法高亮 scope）

**目的**：所有颜色集中管理，支持深浅切换 + 预留 lsh scope 映射。

**新建文件**：`crates/app/src/theme.rs`

```rust
pub struct Theme {
    pub name: String,
    pub is_dark: bool,
    pub background: [f32; 4],
    pub gutter_bg: [f32; 4],
    pub status_bar_bg: [f32; 4],
    pub status_bar_fg: [f32; 4],
    pub line_number: [f32; 4],
    pub selection: [f32; 4],
    pub cursor: [f32; 4],
    pub foreground: [f32; 4],
    pub scopes: HashMap<String, [f32; 4]>,
}

impl Theme {
    pub fn dark() -> Self { ... }
    pub fn light() -> Self { ... }
    /// 从 winit Theme 选对应主题
    pub fn from_winit(theme: winit::window::Theme) -> Self { ... }
    /// 按 scope 名查颜色，fallback 到 foreground
    pub fn scope_color(&self, name: &str) -> [f32; 4] { ... }
}
```

**波及文件**：

| 文件 | 改动 |
|---|---|
| `crates/app/src/theme.rs` | **新增** |
| `crates/app/src/app.rs` | 新增 `current_theme: Theme` 字段；监听 `WindowEvent::ThemeChanged`；所有硬编码色值替换为 `self.current_theme.*` |
| `crates/app/src/render_pipeline.rs` | `shape_visible_lines`、`status_bar_text_vertices` 接受 `&Theme` 参数 |
| `crates/app/src/render_geom.rs` | `compute_selection_highlight_quads` 选区色参数化 |

**验收**：
- 启动时跟随系统主题（macOS 系统设置 → 外观 → 深色/浅色）
- 运行时切换系统外观 → 编辑器实时跟随
- 所有 UI 元素（背景、文字、行号、状态栏、选区、光标）颜色正确

---

### 9.2 多 Document 模型

**目的**：`App.doc_view: Option<DocumentView>` → `Vec<DocumentView>` + 活动索引。

**波及文件**：

| 文件 | 改动 |
|---|---|
| `crates/app/src/app.rs` | `doc_views: Vec<DocumentView>` + `active_index: usize`；所有读/写 `doc_view` 的地方改为走 `self.active_doc()` / `self.active_doc_mut()` |

**关键接口**：

```rust
impl App {
    fn active_doc(&self) -> Option<&DocumentView> {
        self.doc_views.get(self.active_index)
    }
    fn active_doc_mut(&mut self) -> Option<&mut DocumentView> {
        self.doc_views.get_mut(self.active_index)
    }
    fn open_file(&mut self, path: &Path) -> Result<usize, String> { ... }
    fn close_tab(&mut self, index: usize) -> Result<(), String> { ... }
    fn switch_to(&mut self, index: usize) { ... }
}
```

**验收**：
- `cargo check` 编译通过
- 现有测试不退化（480 passed）

---

### 9.3 Tab Bar 渲染

**目的**：窗口顶部渲染 Tab 条（GPU 绘制矩形 + cosmic-text 渲染标签文字）。

**新建文件**：`crates/app/src/tab_bar.rs`

```rust
pub struct TabBarLayout {
    pub tabs: Vec<TabEntry>,    // 每个 tab 的矩形区域
    pub overflow: bool,         // 是否有溢出箭头
    pub scroll_offset: f32,     // overflow 时的水平滚动
}

pub struct TabEntry {
    pub index: usize,
    pub title: String,          // 文件名
    pub dirty: bool,            // 脏标记圆点
    pub rect: Rect,             // 点击区域 (NDC)
    pub close_rect: Rect,       // 关闭按钮区域
}

pub fn layout_tabs(
    doc_views: &[DocumentView],
    active_index: usize,
    screen_w: f32,
    tab_height: f32,
    font_size: f32,
) -> TabBarLayout { ... }

pub fn tab_bar_vertices(
    layout: &TabBarLayout,
    theme: &Theme,
    screen_w: f32,
    screen_h: f32,
) -> Vec<GlyphVertex> { ... }
```

**波及文件**：

| 文件 | 改动 |
|---|---|
| `crates/app/src/tab_bar.rs` | **新增** |
| `crates/app/src/app.rs` | 渲染管线新增 `tab_bar_vertices()` 调用；viewport 的 `visible_rows` 扣除 tab bar 高度 |

**验收**：
- 单 tab 时显示文件名（含 dirty 圆点）
- 多 tab 时水平排列；溢出时显示滚动
- 活动 tab 背景色区别于非活动 tab
- 100 tab 布局耗时 < 4 ms

---

### 9.4 Tab Bar 交互

**目的**：点击切换、关闭按钮、拖拽重排。

**波及文件**：

| 文件 | 改动 |
|---|---|
| `crates/app/src/tab_bar.rs` | `hit_test(x, y)` → 哪个 tab / 关闭按钮 |
| `crates/app/src/app.rs` | `MouseInput` 事件增加 tab bar 区域判断；`CursorMoved` 更新拖拽状态 |

**实现要点**：
- 点击 tab → `switch_to(index)`
- 点击关闭按钮 → `close_tab(index)`
- 拖拽 tab → 更新顺序 + 实时重排
- 鼠标悬浮关闭按钮 → 高亮 + 光标变 pointer

**验收**：
- 点击切换正常
- 关闭非 dirty tab 直接关
- 关闭 dirty tab → 弹出三按钮对话框（Save / Don't Save / Cancel）
- 拖拽重排后顺序持久（本次会话内）

---

### 9.5 文件对话框 + 快捷键

**目的**：Cmd+O 弹出 macOS 原生打开面板；快捷键管理多 tab。

**新增依赖**：`rfd = "0.15"`（跨平台文件对话框，macOS 上走 NSOpenPanel）

**波及文件**：

| 文件 | 改动 |
|---|---|
| `crates/app/Cargo.toml` | 新增 `rfd` |
| `crates/app/src/app.rs` | 处理 `Cmd+O`、`Cmd+T`、`Cmd+W`、`Cmd+Shift+T`、`Cmd+1..9` |

**快捷键表**：

| 快捷键 | 命令 | 行为 |
|---|---|---|
| `Cmd+O` | Open | NSOpenPanel → 选文件 → 新 tab |
| `Cmd+T` | New | 新建空 buffer tab |
| `Cmd+W` | Close | 关当前 tab（dirty 时弹确认） |
| `Cmd+Shift+T` | Reopen | 恢复最近关闭的 tab |
| `Cmd+1..9` | Switch | 跳到第 N 个 tab |
| `Cmd+Shift+[` | Prev | 前一个 tab |
| `Cmd+Shift+]` | Next | 后一个 tab |

**验收**：
- Cmd+O 弹出 macOS 原生文件对话框
- Cmd+T 新建空 tab（标题 "untitled"）
- Cmd+W 关闭 tab，dirty 时三按钮确认
- Cmd+Shift+T 恢复最近关闭
- Cmd+1..9 跳转正确；越界忽略

---

### 9.6 拖拽文件到窗口

**目的**：从 Finder 拖文件到编辑器窗口 → 全部打开。

**实现**：winit 的 `WindowEvent::DroppedFile` / `WindowEvent::HoveredFile`。

**波及文件**：

| 文件 | 改动 |
|---|---|
| `crates/app/src/app.rs` | `WindowEvent::DroppedFile` → `open_file(path)` |

**验收**：
- 拖 1 个文件 → 新 tab 并聚焦
- 拖 5 个文件 → 5 个 tab，聚焦第一个
- 拖已打开的路径 → 聚焦已有 tab（不去重打开）

---

### 9.7 性能验证

**目的**：确保多 buffer 不退化。

**波及文件**：

| 文件 | 改动 |
|---|---|
| `crates/app/benches/tab_bench.rs` | **新增** — bench 100 tab 布局 / 切换 / 重绘 |

**指标**：

| 指标 | 目标 |
|---|---|
| 100 buffer 内存增量 | < 50 MB |
| Tab 切换重绘 | < 16 ms |
| 100 tab 布局 | < 4 ms |

---

## 数据流

```
                    ┌─────────────┐
                    │   App       │
                    │             │
User Input ────────→│ active_idx  │──→ Render Pipeline
(Cmd+O, Click, etc) │             │    (text, tabs,
                    │ doc_views[] │     status_bar)
                    │ theme       │
                    │ tab_history │
                    └─────────────┘
```

- `doc_views: Vec<DocumentView>` — 所有打开的文档
- `active_index: usize` — 当前活动文档索引
- `tab_history: Vec<usize>` — 最近关闭的 tab 索引（用于 Cmd+Shift+T）
- `theme: Theme` — 当前主题（跟随系统）

## 接口设计

### DocumentView 不变

`DocumentView` 保持现有接口不变。多文档管理完全在 `App` 层。

### open_file

```rust
fn open_file(&mut self, path: &Path) -> Result<usize, String>
```

- 如果 `path` 已在 `doc_views` 中 → 返回已有索引，不重复打开
- 否则创建新 `DocumentView::from_file(path)` → push → 返回新索引
- HFS+ case-insensitive：比较前 normalize 路径

### close_tab

```rust
fn close_tab(&mut self, index: usize) -> CloseResult
```

- dirty → 返回 `CloseResult::NeedsConfirm`（由调用方处理对话框）
- clean → 移除 tab，调整 `active_index`，push 到 `tab_history`

---

## 测试计划

### 自动化

| 测试 | 覆盖 |
|---|---|
| `open_duplicate_path_focuses_existing` | 同文件不重复打开 |
| `close_active_switches_to_neighbor` | 关活动 tab 后激活右侧邻居 |
| `close_last_tab_switches_to_left` | 关最后一个 tab 时激活左侧 |
| `close_dirty_returns_needs_confirm` | dirty tab 关闭返回确认 |
| `close_clean_removes_without_prompt` | clean tab 直接关 |
| `recent_closed_restore` | Cmd+Shift+T 恢复 |
| `switch_cmd_1_to_9` | 快捷键跳转 |
| `tab_layout_overflow_scroll` | 溢出布局正确 |
| `tab_layout_truncate_long_name` | 长文件名截断 + "…" |
| `theme_switches_on_system_event` | 系统主题切换事件正确触发 |

### 手动

见 plans.md §9 手动块 + `docs/manual_test_protocol.md`。
