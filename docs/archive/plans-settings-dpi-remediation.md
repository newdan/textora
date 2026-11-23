# Settings / DPI Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 清除 app 生产路径的默认 Settings 回读，修复 Tabs、Retina、Markdown、Workspace viewport 与 Zoom 的实例配置传播。

**Architecture:** `App` 继续持有唯一运行时 Settings；App 方法直接读取实例字段，非 App 模块通过 `ViewportDimensions`、`MarkdownRenderSettings` 或标量参数接收最小输入。Workspace API 采用“先新增显式入口、再迁移调用方、最后删除兼容入口”的三段式迁移，保证每次提交均可编译。

**Tech Stack:** Rust、winit、现有 `Settings`/`UiMetrics`/`Workspace`/`DocumentView`、app 与 ui 单元测试。

---

**设计依据：** `docs/superpowers/specs/2026-06-20-settings-dpi-remediation-design.md`

**共同验收约束：**

- 每个任务最多修改 3 个文件。
- 每个行为修复先看到定向测试失败，再写实现。
- 每次提交前运行该任务定向测试与 `cargo check -p edit-plus-app`。
- 不修改 Phase 3 `AppEffect`、公共 API 收缩和 core 重复测试名。

## 文件职责映射

- `crates/app/src/app.rs`：App 级布局派生值与 viewport 输入构造。
- `crates/app/src/app_dispatch.rs`：preview hit-test 偏移。
- `crates/app/src/app_scroll.rs`：滚轮、翻页、TOC 与光标滚动。
- `crates/app/src/dispatch/mouse.rs`：编辑区鼠标后光标可见性。
- `crates/app/src/app_init.rs`：持久化设置装配、reshape worker 初始化、display map 初始化。
- `crates/app/src/app_renderer.rs`：gutter 与 Markdown preview 输入组装。
- `crates/app/src/workspace.rs`：tab/domain 状态和显式 viewport 输入消费。
- `crates/app/src/md_preview.rs`：Markdown 布局、缓存和 TOC heading 收集。
- `crates/app/src/document_view/visible.rs`：可见行兼容 API。
- `crates/app/src/app_reshape.rs`：Zoom 的逻辑/物理字号换算。
- `crates/app/src/ui_shell.rs`：ShellInputs 与 Dock 布局。

### Task 1: 修复 Tabs 高度与 preview offset 的实例读取

**Files:**
- Modify: `crates/app/src/app.rs:145-188`
- Modify: `crates/app/src/app_dispatch.rs:449-456`
- Test: `crates/app/src/app_tests.rs`

- [ ] **Step 1: 写 Tabs + DPI 哨兵失败测试**

在 `app_tests.rs` 增加：

```rust
#[test]
fn tabs_geometry_and_preview_offset_use_instance_settings() {
    let mut app = app_with_content(vec!["first"]);
    let second = DocumentView::new(vec!["second".into()], 10, 10.0);
    app.workspace.push_view_for_test(crate::view::View::Editor(second));
    app.workspace.switch_to(1);
    app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
    app.settings.dpi_scale = 2.0;

    assert_eq!(app.current_tab_bar_height(), 64.0);
    assert_eq!(app.content_top_offset(), 64.0);
    let (_, preview_y) = app.preview_offsets();
    assert_eq!(preview_y, 96.0);

    app.settings.view_mode = ui::view_mode::ViewMode::Sidebar;
    assert_eq!(app.current_tab_bar_height(), 0.0);
}
```

- [ ] **Step 2: 运行测试并确认失败**

Run:

```bash
cargo test -p edit-plus-app --lib app::app_tests::tabs_geometry_and_preview_offset_use_instance_settings -- --exact
```

Expected: FAIL；Tabs 高度为 `0.0` 或 preview Y 少 16px。

- [ ] **Step 3: 在 App 内计算 tab bar 高度**

将 `content_top_offset` 的 Tabs 分支改为调用 App 自身方法，并替换 `current_tab_bar_height`：

```rust
let tbh = self.current_tab_bar_height();
```

```rust
pub(crate) fn current_tab_bar_height(&self) -> f32 {
    if self.settings.view_mode == ui::view_mode::ViewMode::Tabs && self.workspace.len() > 1 {
        ui::tab_bar::tab_bar_height(self.settings.dpi_scale)
    } else {
        0.0
    }
}
```

将 `preview_offsets` 中的 DPI 来源改为：

```rust
let preview_top_pad = 16.0 * self.settings.dpi_scale;
```

- [ ] **Step 4: 运行定向测试和编译检查**

Run:

```bash
cargo test -p edit-plus-app --lib app::app_tests::tabs_geometry_and_preview_offset_use_instance_settings -- --exact
cargo check -p edit-plus-app
```

Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/app/src/app.rs crates/app/src/app_dispatch.rs crates/app/src/app_tests.rs
git commit -m "fix(app): derive tab geometry from instance settings"
```

### Task 2: 修复滚动与鼠标路径的实例行高和 DPI

**Files:**
- Modify: `crates/app/src/app_scroll.rs`
- Modify: `crates/app/src/dispatch/mouse.rs:60-74`

- [ ] **Step 1: 在 app_scroll 内写 PixelDelta 失败测试**

在 `app_scroll.rs` 末尾增加：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_view::DocumentView;
    use crate::snap_tree::DisplayLineEntry;
    use crate::view::View;
    use winit::dpi::PhysicalPosition;

    #[test]
    fn pixel_scroll_uses_instance_line_height() {
        let mut app = App::new(None);
        app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
        app.settings.line_height = 36.0;
        app.settings.dpi_scale = 2.0;
        app.mouse.pos = (400.0, 300.0);

        let dv = DocumentView::new(
            (0..100).map(|i| format!("line {i}")).collect(),
            10,
            10.0,
        );
        app.workspace.push_view_for_test(View::Editor(dv));
        app.workspace.view_mut(0).unwrap().doc_mut().display.display_map.set_entries(
            (0..100)
                .map(|i| DisplayLineEntry::placeholder(i * 8, 8, 0, 1))
                .collect(),
        );

        app.handle_scroll(MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -36.0)));

        let scroll_top = app.workspace.active_doc().unwrap().display.viewport.scroll_top;
        assert!((scroll_top - 1.0).abs() < 0.01, "scroll_top={scroll_top}");
    }
}
```

- [ ] **Step 2: 运行测试并确认失败**

```bash
cargo test -p edit-plus-app --lib app_scroll::tests::pixel_scroll_uses_instance_line_height -- --exact
```

Expected: FAIL；`scroll_top` 按默认 24.27 行高计算。

- [ ] **Step 3: 一次提取 App 实例标量并替换全部默认读取**

在需要可变借用 Workspace 前提取：

```rust
let dpi = self.settings.dpi_scale;
let line_height = self.settings.line_height;
let view_mode = self.settings.view_mode;
let toc_width = self.settings.toc_width;
```

按以下规则替换 `app_scroll.rs`：

```rust
dpi_scale: dpi
dv.ensure_cursor_visible(line_height)
dv.page_up(line_height)
dv.page_down(line_height)
matches!(view_mode, ui::view_mode::ViewMode::Sidebar)
let toc_w = toc_width // toc_width 已经是物理像素，不再乘 dpi
let preview_top_pad = 16.0 * dpi
```

所有 `scroll_pixels`、`clamp_anchor`、`derive_scroll_top` 使用同一个 `line_height`。

在 `dispatch/mouse.rs` 的非 preview 分支借用 Workspace 前保存：

```rust
let line_height = self.settings.line_height;
```

并改为：

```rust
dv.ensure_cursor_visible(line_height);
```

- [ ] **Step 4: 验证无生产默认读取并运行测试**

```bash
rg -n "Settings::new" crates/app/src/app_scroll.rs crates/app/src/dispatch/mouse.rs
cargo test -p edit-plus-app --lib app_scroll::tests::pixel_scroll_uses_instance_line_height -- --exact
cargo test -p edit-plus-app --lib app::app_tests::move_down_wrapped_line_boundary_no_stall -- --exact
cargo check -p edit-plus-app
```

Expected: `rg` 无输出；测试和 check PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/app/src/app_scroll.rs crates/app/src/dispatch/mouse.rs
git commit -m "fix(app): use instance metrics for scrolling"
```

### Task 3: 先装配持久化 Settings，再创建 reshape worker

**Files:**
- Modify: `crates/app/src/app_init.rs:21-66,138-315`

- [ ] **Step 1: 写持久化设置与 display map 哨兵测试**

在 `app_init.rs` 末尾增加：

```rust
#[cfg(test)]
mod settings_tests {
    use super::*;
    use crate::document_view::DocumentView;
    use crate::view::View;

    #[test]
    fn settings_from_persisted_preserves_font_configuration() {
        let persisted = crate::settings_io::PersistedSettings {
            font_family: "Audit Mono".into(),
            font_size: 19.0,
            line_height_ratio: 1.5,
            ..Default::default()
        };
        let settings = settings_from_persisted(&persisted);
        assert_eq!(settings.font_family, "Audit Mono");
        assert_eq!(settings.font_size, 19.0);
        assert_eq!(settings.line_height, 28.5);
    }

    #[test]
    fn init_display_map_uses_instance_font_size() {
        let mut app = App::new(None);
        app.settings.font_size = 31.0;
        app.settings.line_height = 44.0;
        let dv = DocumentView::new(vec!["sentinel".into()], 10, 10.0);
        app.workspace.push_view_for_test(View::Editor(dv));

        app.init_display_map(0);

        let snapshot = app.workspace.view(0).unwrap().doc().display.display_map.snapshot();
        assert_eq!(snapshot.font_size, 31.0);
    }
}
```

- [ ] **Step 2: 运行两个测试并确认失败**

```bash
cargo test -p edit-plus-app --lib app_init::settings_tests::settings_from_persisted_preserves_font_configuration -- --exact
cargo test -p edit-plus-app --lib app_init::settings_tests::init_display_map_uses_instance_font_size -- --exact
```

Expected: 第一个因 helper 不存在而编译失败；实现测试壳后第二个得到默认 `15.0`。

- [ ] **Step 3: 提取纯 Settings 装配 helper**

在 `App::new` 前增加：

```rust
fn settings_from_persisted(
    persisted: &crate::settings_io::PersistedSettings,
) -> ui::settings::Settings {
    let mut settings = ui::settings::Settings::new();
    settings.view_mode = persisted.view_mode;
    settings.theme_mode = persisted.theme_mode;
    settings.show_line_numbers = persisted.show_line_numbers;
    settings.word_wrap = persisted.word_wrap;
    settings.show_status_bar = persisted.show_status_bar;
    settings.font_family = persisted.font_family.clone();
    settings.font_size = persisted.font_size;
    settings.line_height_ratio = persisted.line_height_ratio;
    settings.line_height = persisted.font_size * persisted.line_height_ratio;
    settings.tab_width = persisted.tab_width;
    settings
}
```

把 `settings_io::load()` 和 `settings_from_persisted()` 移到 FontSystem/worker 创建之前，然后使用：

```rust
let font_family = settings.font_family.clone();
let font_size = settings.font_size;
```

创建 worker。删除原来 worker 之后重复的 Settings 装配代码。

- [ ] **Step 4: 让 init_display_map 只读 self.settings**

在 `init_display_map` 开头复制：

```rust
let font_size = self.settings.font_size;
let line_height = self.settings.line_height;
```

把 content hash、anchor、预整形和最终 `set_viewport_size` 的默认 font/line height 全部替换为这两个值；最终阶段不得再次声明默认 `font_size`。

- [ ] **Step 5: 验证并提交**

```bash
rg -n "Settings::new" crates/app/src/app_init.rs
cargo test -p edit-plus-app --lib app_init::settings_tests:: -- --nocapture
cargo check -p edit-plus-app
git add crates/app/src/app_init.rs
git commit -m "fix(app): initialize shaping from persisted settings"
```

Expected: `rg` 只命中 `settings_from_persisted` 中唯一合法根构造；测试与 check PASS。

### Task 4: 修复 gutter 对实例设置的读取

**Files:**
- Modify: `crates/app/src/app_renderer.rs:12-35`
- Test: `crates/app/src/app_tests.rs`

- [ ] **Step 1: 写隐藏行号失败测试**

```rust
#[test]
fn editor_left_margin_respects_instance_line_number_setting() {
    let mut app = app_with_content(vec!["line"]);
    app.settings.dpi_scale = 1.0;
    app.settings.show_line_numbers = false;
    app.settings.font_size = 40.0;

    assert_eq!(app.editor_left_margin(1_000_000), 32.0);

    app.settings.show_line_numbers = true;
    assert!(app.editor_left_margin(1_000_000) > 32.0);
}
```

- [ ] **Step 2: 运行测试并确认失败**

```bash
cargo test -p edit-plus-app --lib app::app_tests::editor_left_margin_respects_instance_line_number_setting -- --exact
```

Expected: FAIL；隐藏行号时仍使用默认 gutter。

- [ ] **Step 3: 使用 App Settings 计算 gutter**

```rust
let gutter_w = self.settings.gutter_width(line_count);
let lm = self.settings.content_left_margin().max(gutter_w);
```

- [ ] **Step 4: 验证并提交**

```bash
rg -n "Settings::new" crates/app/src/app_renderer.rs
cargo test -p edit-plus-app --lib app::app_tests::editor_left_margin_respects_instance_line_number_setting -- --exact
cargo check -p edit-plus-app
git add crates/app/src/app_renderer.rs crates/app/src/app_tests.rs
git commit -m "fix(app): derive gutter from instance settings"
```

### Task 5: 新增 Workspace 显式 viewport API 与 App 派生 helper

**Files:**
- Modify: `crates/app/src/workspace.rs`
- Modify: `crates/app/src/app.rs:164-170`
- Test: `crates/app/src/app_tests.rs`

- [ ] **Step 1: 写显式 viewport 输入失败测试**

在 `workspace.rs::tests` 增加：

```rust
#[test]
fn new_tab_with_viewport_uses_supplied_dimensions() {
    let mut ws = Workspace::new();
    let dims = ViewportDimensions { visible_rows: 7, viewport_height: 7.5 };
    ws.new_empty_tab_with_viewport(dims);
    let vp = &ws.active_doc().unwrap().display.viewport;
    assert_eq!(vp.visible_rows, 7);
    assert_eq!(vp.viewport_height, 7.5);
}

#[test]
fn restore_with_viewport_uses_supplied_dimensions() {
    let mut source = Workspace::new();
    source.new_empty_tab_with_viewport(ViewportDimensions {
        visible_rows: 3,
        viewport_height: 3.0,
    });
    let snapshot = source.snapshot(false, None);
    let dims = ViewportDimensions { visible_rows: 9, viewport_height: 9.5 };

    let restored = Workspace::restore_with_viewport(snapshot, dims, 36.0).unwrap();

    let vp = &restored.active_doc().unwrap().display.viewport;
    assert_eq!(vp.visible_rows, 9);
    assert_eq!(vp.viewport_height, 9.5);
}
```

在 `app_tests.rs` 增加：

```rust
#[test]
fn viewport_dimensions_use_instance_line_height_and_chrome() {
    let mut app = App::new(None);
    app.settings.view_mode = ui::view_mode::ViewMode::Tabs;
    app.settings.dpi_scale = 2.0;
    app.settings.line_height = 40.0;
    let dims = app.viewport_dimensions(400.0);
    assert_eq!(dims.visible_rows, 10);
    assert_eq!(dims.viewport_height, 10.0);
}
```

- [ ] **Step 2: 运行测试并确认编译失败**

```bash
cargo test -p edit-plus-app --lib workspace::tests::new_tab_with_viewport_uses_supplied_dimensions -- --exact
cargo test -p edit-plus-app --lib app::app_tests::viewport_dimensions_use_instance_line_height_and_chrome -- --exact
```

Expected: FAIL；类型和方法尚不存在。

- [ ] **Step 3: 定义输入类型和 App helper**

在 `workspace.rs` 的 effect 类型附近增加：

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ViewportDimensions {
    pub(crate) visible_rows: usize,
    pub(crate) viewport_height: f64,
}
```

在 App 中增加：

```rust
pub(crate) fn viewport_dimensions(&self, screen_height: f32) -> ViewportDimensions {
    ViewportDimensions {
        visible_rows: self.visible_rows(screen_height),
        viewport_height: self.visible_height_lines(screen_height),
    }
}
```

并导入：

```rust
use crate::workspace::{ViewportDimensions, Workspace};
```

- [ ] **Step 4: 新增 Workspace 显式入口，保留旧入口作为临时兼容层**

新增：

```rust
pub(crate) fn open_file_with_viewport(
    &mut self,
    path: &Path,
    viewport: ViewportDimensions,
) -> Result<WorkspaceEffect, String>
```

将现有 `open_file` 重命名为 `open_file_with_viewport`，删除函数开头创建 `Settings::new()` 并计算 `(visible_rows, viewport_height)` 的代码块。创建 `DocumentView` 时改为：

```rust
let dv = DocumentView::from_file(path, viewport.visible_rows, viewport.viewport_height)
    .map_err(|e| e.to_string())?;
```

新增：

```rust
pub(crate) fn new_empty_tab_with_viewport(
    &mut self,
    viewport: ViewportDimensions,
) -> WorkspaceEffect {
    let mut dv = DocumentView::new(
        vec![String::new()],
        viewport.visible_rows,
        viewport.viewport_height,
    );
    dv.dirty_snapshot_id =
        Some(crate::dirty_snapshot::snapshot_filename(&crate::dirty_snapshot::untitled_id()));
    self.record_nav_step();
    self.views.push(View::Editor(dv));
    self.active_index = self.views.len() - 1;
    WorkspaceEffect::ActiveTabChanged
}
```

新增：

```rust
pub(crate) fn restore_with_viewport(
    snap: PersistedWorkspace,
    viewport: ViewportDimensions,
    line_height: f32,
) -> std::io::Result<Self>
```

将现有 `restore` 重命名为 `restore_with_viewport`，删除函数开头计算 `(visible_rows, viewport_height)` 的代码块，并把函数体内构造 DocumentView 的两个参数分别替换为 `viewport.visible_rows`、`viewport.viewport_height`。第 709-711 行 stub anchor 以及第 730 行 `ensure_cursor_visible` 全部使用传入的 `line_height`。

旧 `open_file/new_empty_tab/restore` 暂时委托新入口，以保证调用方分批迁移期间可编译。

- [ ] **Step 5: 验证并提交**

```bash
cargo test -p edit-plus-app --lib workspace::tests::new_tab_with_viewport_uses_supplied_dimensions -- --exact
cargo test -p edit-plus-app --lib app::app_tests::viewport_dimensions_use_instance_line_height_and_chrome -- --exact
cargo check -p edit-plus-app
git add crates/app/src/workspace.rs crates/app/src/app.rs crates/app/src/app_tests.rs
git commit -m "refactor(app): define explicit workspace viewport input"
```

### Task 6: 迁移 dispatcher 的 Workspace 调用

**Files:**
- Modify: `crates/app/src/dispatch/tabs.rs`
- Modify: `crates/app/src/dispatch/commands.rs`
- Modify: `crates/app/src/app_dispatch.rs`

- [ ] **Step 1: 先把一个调用改成显式 API 并确认类型链**

在 `dispatch/tabs.rs::open_file` 中先保存：

```rust
let viewport = self.viewport_dimensions(self.screen_height());
let effect = self.workspace.open_file_with_viewport(path, viewport)?;
```

Run:

```bash
cargo check -p edit-plus-app
```

Expected: PASS，证明新入口可被 App handler 使用。

- [ ] **Step 2: 迁移 tabs 新建文档**

```rust
let viewport = self.viewport_dimensions(self.screen_height());
let effect = self.workspace.new_empty_tab_with_viewport(viewport);
```

- [ ] **Step 3: 迁移 commands 的三处文件打开**

在每个调用前单独计算，避免同时借用 `self.workspace` 与 `self`。对 OpenRecentFile 分支只做下面两行替换：

```rust
let viewport = self.viewport_dimensions(self.screen_height());
match self.workspace.open_file_with_viewport(&path, viewport) {
```

该行后继续使用原有 `Ok(ws_effect)` 和 `Err(e)` match arms；从光标恢复到 `handle_workspace_effect` 的语句不移动、不增删。OpenSettings 分支同样使用 `open_file_with_viewport`。

OpenSettings 分支同样使用 `open_file_with_viewport`。

- [ ] **Step 4: 迁移 app_dispatch 的 OpenSettingsFile**

```rust
let viewport = self.viewport_dimensions(self.screen_height());
if let Err(e) = self.workspace.open_file_with_viewport(&path, viewport) {
    eprintln!("Failed to open settings.toml: {e}");
}
```

- [ ] **Step 5: 验证并提交**

```bash
rg -n "workspace\.(open_file|new_empty_tab)\(" \
  crates/app/src/dispatch/tabs.rs \
  crates/app/src/dispatch/commands.rs \
  crates/app/src/app_dispatch.rs
cargo test -p edit-plus-app --lib workspace::tests::
cargo check -p edit-plus-app
git add crates/app/src/dispatch/tabs.rs crates/app/src/dispatch/commands.rs crates/app/src/app_dispatch.rs
git commit -m "refactor(app): pass viewport input through dispatchers"
```

Expected: `rg` 无旧入口输出；测试和 check PASS。

### Task 7: 迁移窗口恢复并删除 Workspace 默认兼容入口

**Files:**
- Modify: `crates/app/src/app_window.rs`
- Modify: `crates/app/src/workspace.rs`

- [ ] **Step 1: 迁移 workspace restore**

在 `init_window` 中改为：

```rust
let viewport = self.viewport_dimensions(screen_h);
let line_height = self.settings.line_height;
if let Ok(restored) = crate::workspace::Workspace::restore_with_viewport(
    restored_snap,
    viewport,
    line_height,
) {
    self.workspace = restored;
    self.update_tab_layout(true);
    self.file_path = None;
    if !self.workspace.is_empty() {
        self.init_display_map(self.workspace.active_index());
    }
}
```

- [ ] **Step 2: 迁移 app_window 测试夹具**

每个 `app.workspace.new_empty_tab(600.0)` 改为两步：

```rust
let viewport = app.viewport_dimensions(600.0);
app.workspace.new_empty_tab_with_viewport(viewport);
```

- [ ] **Step 3: 迁移 workspace.rs 内部测试夹具**

在 `workspace.rs::tests` 增加：

```rust
fn test_viewport() -> ViewportDimensions {
    ViewportDimensions { visible_rows: 22, viewport_height: 22.0 }
}
```

把该模块的 `ws.new_empty_tab(600.0)` 全部改为：

```rust
ws.new_empty_tab_with_viewport(test_viewport());
```

- [ ] **Step 4: 删除旧入口和 Workspace tab 高度方法**

删除以下三个默认 Settings 兼容入口：

```rust
Workspace::open_file(path, screen_height)
Workspace::new_empty_tab(screen_height)
Workspace::restore(snap, screen_height)
```

删除已无调用的：

```rust
Workspace::current_tab_bar_height()
```

- [ ] **Step 5: 验证并提交**

```bash
rg -n "Settings::new|current_tab_bar_height" crates/app/src/workspace.rs
rg -n "Workspace::restore\(|workspace\.(open_file|new_empty_tab)\(" crates/app/src -g '*.rs'
cargo test -p edit-plus-app --lib workspace::tests::
cargo test -p edit-plus-app --lib app_window::
cargo check -p edit-plus-app
git add crates/app/src/app_window.rs crates/app/src/workspace.rs
git commit -m "refactor(app): remove workspace settings fallbacks"
```

Expected: 两个 `rg` 均无旧生产入口输出；测试和 check PASS。

### Task 8: 给 Markdown preview 注入显式渲染配置

**Files:**
- Modify: `crates/app/src/md_preview.rs:137-204,269-370`
- Modify: `crates/app/src/app_renderer.rs:420-459`

- [ ] **Step 1: 写 Markdown 字体和 TOC 深度失败测试**

在 `md_preview.rs::heading_tests` 增加：

```rust
#[test]
fn render_settings_control_style_and_toc_depth() {
    let theme = Theme::from_definition(&ui::theme::ThemeDefinition::default_dark());
    let settings = MarkdownRenderSettings {
        font_size: 36.0,
        line_height: 58.0,
        toc_max_depth: 5,
    };
    let style = settings.style(&theme);
    assert_eq!(style.body_font_size, 36.0);
    assert_eq!(style.line_height, 58.0);

    let mut preview = MarkdownPreview::new();
    preview.set_source("# H1\n\n##### H5".into(), 1);
    let _ = preview.render(&theme, 600.0, 400.0, 0.0, 0.0, settings, None);
    assert_eq!(preview.headings().len(), 2);
    assert_eq!(preview.headings()[1].level, 5);
}
```

- [ ] **Step 2: 运行测试并确认编译失败**

```bash
cargo test -p edit-plus-app --lib md_preview::heading_tests::render_settings_control_style_and_toc_depth -- --exact
```

Expected: FAIL；`MarkdownRenderSettings` 和新 render 参数尚不存在。

- [ ] **Step 3: 定义配置并贯穿 render/heading**

在 `MarkdownPreview` 前增加：

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MarkdownRenderSettings {
    pub(crate) font_size: f32,
    pub(crate) line_height: f32,
    pub(crate) toc_max_depth: u8,
}

impl MarkdownRenderSettings {
    fn style(self, theme: &Theme) -> MarkdownStyle {
        MarkdownStyle::from_theme(theme, self.font_size, self.line_height)
    }
}
```

签名改为：

```rust
fn collect_headings(&mut self, max_depth: u8)
```

并以 `max_depth` 过滤 heading。`render` 增加 `settings: MarkdownRenderSettings` 参数，使用：

```rust
let style = settings.style(theme);
```

两处 `collect_headings()` 都改为：

```rust
self.collect_headings(settings.toc_max_depth);
```

- [ ] **Step 4: App renderer 构造同一份配置**

在取得 `active_view_mut()` 前创建：

```rust
let preview_settings = crate::md_preview::MarkdownRenderSettings {
    font_size: self.settings.font_size,
    line_height: self.settings.line_height,
    toc_max_depth: self.settings.toc_max_depth,
};
```

render 调用增加 `preview_settings`，位置在 `offset_y` 与 `shaper` 之间。

- [ ] **Step 5: 验证并提交**

```bash
rg -n "Settings::new" crates/app/src/md_preview.rs
cargo test -p edit-plus-app --lib md_preview::heading_tests::render_settings_control_style_and_toc_depth -- --exact
cargo test -p edit-plus-app --lib md_preview::heading_tests::
cargo check -p edit-plus-app
git add crates/app/src/md_preview.rs crates/app/src/app_renderer.rs
git commit -m "refactor(app): inject markdown render settings"
```

### Task 9: 为 DocumentView 增加显式 line-height visible API

**Files:**
- Modify: `crates/app/src/document_view/visible.rs`
- Test: `crates/app/src/document_view/basic_tests.rs`

- [ ] **Step 1: 写非默认行高失败测试**

在 `basic_tests.rs` 增加：

```rust
#[test]
fn visible_range_with_line_height_uses_explicit_value() {
    let mut dv = DocumentView::new(make_lines(20), 4, 4.0);
    dv.display.display_map.set_entries(
        (0..20)
            .map(|i| {
                let visual_lines = if i == 3 { 3 } else { 1 };
                crate::snap_tree::DisplayLineEntry::placeholder(
                    i * 8,
                    8,
                    0,
                    visual_lines,
                )
            })
            .collect(),
    );
    dv.display.viewport.scroll_anchor = ui::viewport::ScrollAnchor::new(3, 50.0);
    let range = dv.visible_doc_range_with_line_height(36.0);
    assert_eq!(range, 3..7);
}
```

- [ ] **Step 2: 运行测试并确认编译失败**

```bash
cargo test -p edit-plus-app --lib document_view::basic_tests::visible_range_with_line_height_uses_explicit_value -- --exact
```

Expected: FAIL；显式方法尚不存在。

- [ ] **Step 3: 增加显式方法族，暂时保留旧方法**

核心范围方法：

```rust
pub(crate) fn visible_doc_range_with_line_height(
    &self,
    line_height: f32,
) -> std::ops::Range<usize> {
    let total = self.line_index.line_count();
    if total == 0 {
        return 0..0;
    }
    if self.display.display_map.line_count() == total {
        self.display.viewport.visible_doc_range_from_anchor(
            &self.display.display_map,
            line_height,
        )
    } else {
        let max_start = total.saturating_sub(1);
        let start = (self.display.viewport.scroll_top.floor() as usize).min(max_start);
        let visible_count = self.display.viewport.visible_rows.max(1);
        let end = (start + visible_count).min(total);
        start..end
    }
}
```

增加以下显式入口。先抽取当前 `visible_line` 的字节读取主体：

```rust
fn visible_line_in_range(
    &self,
    vis_idx: usize,
    range: std::ops::Range<usize>,
) -> Option<Cow<'_, [u8]>> {
    let doc_idx = range.start + vis_idx;
    if doc_idx >= range.end || doc_idx >= self.line_index.offsets.len() {
        return None;
    }
    let offset = self.line_index.offsets[doc_idx];
    let length = self.line_index.lengths[doc_idx];
    if length == 0 {
        return Some(Cow::Borrowed(&[]));
    }
    let total = self.tb.text_length();
    if offset >= total {
        return Some(Cow::Borrowed(&[]));
    }
    let chunk = self.tb.read_forward(offset);
    if chunk.len() >= length {
        return Some(Cow::Borrowed(&chunk[..length]));
    }
    let mut result = Vec::with_capacity(length);
    let mut i = offset;
    while result.len() < length && i < total {
        let chunk = self.tb.read_forward(i);
        if chunk.is_empty() {
            break;
        }
        let take = (length - result.len()).min(chunk.len());
        result.extend_from_slice(&chunk[..take]);
        i += take;
    }
    Some(Cow::Owned(result))
}
```

再增加完整的显式方法族：

```rust
pub fn visible_line_with_line_height(
    &self,
    vis_idx: usize,
    line_height: f32,
) -> Option<Cow<'_, [u8]>> {
    self.visible_line_in_range(
        vis_idx,
        self.visible_doc_range_with_line_height(line_height),
    )
}

pub fn visible_line_wrap_with_line_height(
    &self,
    vis_idx: usize,
    line_height: f32,
) -> Option<Cow<'_, [u8]>> {
    self.visible_line_with_line_height(vis_idx, line_height)
}

pub fn visible_lines_with_line_height(&self, line_height: f32) -> Vec<String> {
    let range = self.visible_doc_range_with_line_height(line_height);
    (0..range.len())
        .filter_map(|i| self.visible_line_in_range(i, range.clone()))
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .collect()
}

pub fn visible_line_count_with_line_height(&self, line_height: f32) -> usize {
    let range = self.visible_doc_range_with_line_height(line_height);
    range.len().min(self.line_index.offsets.len())
}

pub fn visible_line_count_wrap_with_line_height(&self, line_height: f32) -> usize {
    self.visible_line_count_with_line_height(line_height)
}

pub fn visible_line_key_with_line_height(
    &self,
    vis_idx: usize,
    line_height: f32,
) -> Option<(usize, usize)> {
    let range = self.visible_doc_range_with_line_height(line_height);
    let doc_idx = range.start + vis_idx;
    if doc_idx >= range.end || doc_idx >= self.line_index.offsets.len() {
        return None;
    }
    Some((self.line_index.offsets[doc_idx], self.line_index.lengths[doc_idx]))
}

pub fn visible_line_key_wrap_with_line_height(
    &self,
    vis_idx: usize,
    line_height: f32,
) -> Option<(usize, usize)> {
    self.visible_line_key_with_line_height(vis_idx, line_height)
}
```

旧方法暂时保留，供测试调用方分批迁移；此任务不改变旧签名。

- [ ] **Step 4: 运行测试与编译检查**

```bash
cargo test -p edit-plus-app --lib document_view::basic_tests::visible_range_with_line_height_uses_explicit_value -- --exact
cargo check -p edit-plus-app
```

Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/app/src/document_view/visible.rs crates/app/src/document_view/basic_tests.rs
git commit -m "refactor(app): add explicit visible line metrics"
```

### Task 10: 迁移 DocumentView 测试调用方

**Files:**
- Modify: `crates/app/src/document_view/basic_tests.rs`
- Modify: `crates/app/src/document_view/boundary_tests.rs`
- Modify: `crates/app/src/document_view/cursor_visual_tests.rs`

- [ ] **Step 1: 在每个文件定义一致测试行高**

```rust
const TEST_LINE_HEIGHT: f32 = 24.27;
```

- [ ] **Step 2: 迁移 basic_tests 和 boundary_tests**

按以下精确映射替换：

```rust
dv.visible_line(i)
// ->
dv.visible_line_with_line_height(i, TEST_LINE_HEIGHT)

dv.visible_lines()
// ->
dv.visible_lines_with_line_height(TEST_LINE_HEIGHT)

dv.visible_line_count()
// ->
dv.visible_line_count_with_line_height(TEST_LINE_HEIGHT)
```

- [ ] **Step 3: 迁移 cursor_visual_tests**

除上面映射外，再替换：

```rust
dv.visible_line_wrap(i)
// ->
dv.visible_line_wrap_with_line_height(i, TEST_LINE_HEIGHT)

dv.visible_line_count_wrap()
// ->
dv.visible_line_count_wrap_with_line_height(TEST_LINE_HEIGHT)

dv.visible_line_key_wrap(i)
// ->
dv.visible_line_key_wrap_with_line_height(i, TEST_LINE_HEIGHT)
```

- [ ] **Step 4: 验证并提交**

```bash
cargo test -p edit-plus-app --lib document_view::basic_tests::
cargo test -p edit-plus-app --lib document_view::boundary_tests::
cargo test -p edit-plus-app --lib document_view::cursor_visual_tests::
cargo check -p edit-plus-app
git add \
  crates/app/src/document_view/basic_tests.rs \
  crates/app/src/document_view/boundary_tests.rs \
  crates/app/src/document_view/cursor_visual_tests.rs
git commit -m "test(app): pass explicit metrics to visible line tests"
```

### Task 11: 迁移剩余 visible 调用并删除隐式 API

**Files:**
- Modify: `crates/app/src/commands.rs`
- Modify: `crates/app/src/document_view/word_wrap_tests.rs`
- Modify: `crates/app/src/document_view/visible.rs`

- [ ] **Step 1: 迁移 commands 测试调用**

在 `commands.rs` 的测试模块定义：

```rust
const TEST_LINE_HEIGHT: f32 = 24.27;
```

把该测试模块内的 `visible_line/visible_lines` 调用改为对应的 `_with_line_height` 入口。

- [ ] **Step 2: 迁移 word_wrap_tests**

定义相同常量，并把：

```rust
dv.visible_line_count()
```

改为：

```rust
dv.visible_line_count_with_line_height(TEST_LINE_HEIGHT)
```

- [ ] **Step 3: 删除 visible.rs 的旧隐式入口**

删除以下旧方法：

```rust
visible_doc_range()
visible_line()
visible_line_wrap()
visible_lines()
visible_line_count()
visible_line_count_wrap()
visible_line_key()
visible_line_key_wrap()
```

保留 `_with_line_height` 方法名，避免再次修改 Task 10 已迁移的三个测试文件。最终 API 的显式性由方法名和必填参数共同表达。

- [ ] **Step 4: 扫描和验证**

```bash
rg -n "Settings::new" crates/app/src/document_view/visible.rs
rg -n "visible_(line|lines|line_count|line_wrap|line_key)(_with_line_height)?\(" \
  crates/app/src/document_view crates/app/src/commands.rs
cargo test -p edit-plus-app --lib document_view::
cargo test -p edit-plus-app --lib commands::command_tests::
cargo check -p edit-plus-app
```

Expected: 第一条无输出；第二条所有最终调用都显式传入行高；测试和 check PASS。

- [ ] **Step 5: 提交**

```bash
git add \
  crates/app/src/commands.rs \
  crates/app/src/document_view/word_wrap_tests.rs \
  crates/app/src/document_view/visible.rs
git commit -m "refactor(app): require line height for visible line access"
```

### Task 12: 修复 Zoom 的逻辑字号语义

**Files:**
- Modify: `crates/app/src/app_reshape.rs:16-39,282-412`
- Modify: `crates/app/src/dispatch/commands.rs:118-127`
- Test: `crates/ui/src/settings.rs:354-374`

- [ ] **Step 1: 写 Retina Zoom 失败测试**

在 `app_reshape.rs::zoom_tests` 增加：

```rust
#[test]
fn zoom_uses_logical_points_at_retina_scale() {
    let mut app = App::new(None);
    app.settings.apply_scale(2.0);

    app.apply_zoom(16.0);
    assert_eq!(app.settings.font_size, 32.0);
    assert_eq!(app.settings.logical_font_size(), 16.0);

    app.apply_zoom(15.0);
    assert_eq!(app.settings.font_size, 30.0);
    assert_eq!(app.settings.logical_font_size(), 15.0);
}

#[test]
fn zoom_out_clamps_logical_size_at_six() {
    let mut app = App::new(None);
    app.settings.apply_scale(2.0);
    app.apply_zoom(6.0);
    assert_eq!(app.settings.font_size, 12.0);
    assert_eq!(app.settings.logical_font_size(), 6.0);
}
```

在 `crates/ui/src/settings.rs::tests` 增加 DPI 往返覆盖：

```rust
#[test]
fn apply_scale_roundtrip_preserves_logical_metrics() {
    let mut settings = Settings::new();
    settings.apply_scale(2.0);
    settings.apply_scale(1.0);
    settings.apply_scale(2.0);

    assert_eq!(settings.dpi_scale, 2.0);
    assert_eq!(settings.font_size, 30.0);
    assert_eq!(settings.line_height, 48.54);
    assert_eq!(settings.logical_font_size(), 15.0);
    assert_eq!(settings.logical_line_height(), 24.27);
}
```

- [ ] **Step 2: 运行测试并确认失败**

```bash
cargo test -p edit-plus-app --lib app_reshape::zoom_tests::zoom_uses_logical_points_at_retina_scale -- --exact
cargo test -p edit-plus-app --lib app_reshape::zoom_tests::zoom_out_clamps_logical_size_at_six -- --exact
```

Expected: FAIL；物理字号被直接设置为 16/6。

- [ ] **Step 3: 让 apply_zoom 接收逻辑字号**

```rust
pub(crate) fn apply_zoom(&mut self, logical_font_size: f32) {
    let dpi = self.settings.dpi_scale.max(f32::EPSILON);
    let physical_font_size = logical_font_size.clamp(6.0, 1000.0) * dpi;
    self.settings.set_font_size(physical_font_size);
    if let Some(ref mut text) = self.text {
        text.shaper.set_font_size(physical_font_size);
    }
    for i in 0..self.workspace.len() {
        let dv = self.workspace.view_mut(i).unwrap().doc_mut();
        dv.display.render_cache.invalidate_all();
    }
    let lh = self.settings.line_height;
    if let Some(ref gpu) = self.gpu {
        let h = gpu.ctx.config.height as f32;
        let visible_rows = self.visible_rows(h);
        let viewport_height = self.visible_height_lines(h);
        for i in 0..self.workspace.len() {
            let dv = self.workspace.view_mut(i).unwrap().doc_mut();
            dv.resize(visible_rows, viewport_height);
            dv.display.viewport.clamp_anchor(&dv.display.display_map, lh);
            dv.display.viewport.derive_scroll_top(&dv.display.display_map, lh);
        }
    }
    self.invalidate_reshape();
    self.needs_redraw = true;
}
```

更新测试 helper：

```rust
fn sim_zoom_in(app: &mut App) {
    app.apply_zoom(app.settings.logical_font_size() + 1.0);
}

fn sim_zoom_out(app: &mut App) {
    app.apply_zoom((app.settings.logical_font_size() - 1.0).max(6.0));
}

fn sim_zoom_reset(app: &mut App) {
    app.apply_zoom(15.0);
}
```

- [ ] **Step 4: 更新命令入口**

```rust
crate::menu_handler::AppCommand::ZoomIn => {
    self.apply_zoom(self.settings.logical_font_size() + 1.0);
}
crate::menu_handler::AppCommand::ZoomOut => {
    self.apply_zoom((self.settings.logical_font_size() - 1.0).max(6.0));
}
crate::menu_handler::AppCommand::ZoomReset => {
    self.apply_zoom(15.0);
}
```

- [ ] **Step 5: 验证并提交**

```bash
cargo test -p edit-plus-app --lib app_reshape::zoom_tests::
cargo test -p edit-plus-ui --lib settings::tests::apply_scale_roundtrip_preserves_logical_metrics -- --exact
cargo check -p edit-plus-app
git add crates/app/src/app_reshape.rs crates/app/src/dispatch/commands.rs crates/ui/src/settings.rs
git commit -m "fix(app): keep zoom semantics in logical points"
```

### Task 13: 让 ShellInputs 只有一份 DPI 真值

**Files:**
- Modify: `crates/app/src/ui_shell.rs`
- Modify: `crates/app/src/app_window.rs`
- Modify: `crates/app/src/events.rs`

- [ ] **Step 1: 写 metrics.dpi 优先失败测试**

在 `ui_shell.rs::tests` 增加：

```rust
#[test]
fn shell_layout_uses_metrics_dpi() {
    let mut settings = ui::settings::Settings::new();
    settings.apply_scale(2.0);
    let inputs = ShellInputs {
        tabs_visible: false,
        tabs_thickness: 0.0,
        search_visible: false,
        search_thickness: 0.0,
        status_thickness: 0.0,
        sidebar_visible: true,
        sidebar_thickness: 440.0,
        scrollbar_thickness: 0.0,
        toc_visible: false,
        toc_thickness: 0.0,
        metrics: ui::settings::UiMetrics::from(&settings),
        dpi: 1.0,
    };
    assert_eq!(run_layout(&inputs).y, 72.0);
}
```

- [ ] **Step 2: 运行测试并确认失败**

```bash
cargo test -p edit-plus-app --lib ui_shell::tests::shell_layout_uses_metrics_dpi -- --exact
```

Expected: FAIL；布局使用独立 `dpi=1.0`，得到 36px title bar。

- [ ] **Step 3: 删除 ShellInputs.dpi 并统一读取 metrics**

结构改为：

```rust
pub struct ShellInputs {
    pub tabs_visible: bool,
    pub tabs_thickness: f32,
    pub search_visible: bool,
    pub search_thickness: f32,
    pub status_thickness: f32,
    pub sidebar_visible: bool,
    pub sidebar_thickness: f32,
    pub scrollbar_thickness: f32,
    pub toc_visible: bool,
    pub toc_thickness: f32,
    pub metrics: ui::settings::UiMetrics,
}
```

替换：

```rust
let dpi = inputs.metrics.dpi;
title_bar_height(inputs.metrics.dpi)
toc.set_scroll_y(self.toc_scroll_y, inputs.metrics.dpi)
```

删除 `ui_shell.rs` 所有测试构造中的 `dpi:` 字段。

- [ ] **Step 4: 更新 App、events 和测试构造**

在 `app_window.rs::build_shell_inputs` 使用：

```rust
metrics: ui::settings::UiMetrics::from(&self.settings),
```

删除独立 `dpi` 字段和 `self.settings.clone()`。更新 `app_window.rs`、`events.rs` 中全部 `ShellInputs` 构造，删除 `dpi:`。

- [ ] **Step 5: 验证并提交**

```bash
rg -n "self\.settings\.clone\(\)|inputs\.dpi|pub dpi: f32" \
  crates/app/src/ui_shell.rs crates/app/src/app_window.rs crates/app/src/events.rs
cargo test -p edit-plus-app --lib ui_shell::tests::shell_layout_uses_metrics_dpi -- --exact
cargo test -p edit-plus-app --lib ui_shell::tests::
cargo test -p edit-plus-app --lib app_window::
cargo check -p edit-plus-app
git add crates/app/src/ui_shell.rs crates/app/src/app_window.rs crates/app/src/events.rs
git commit -m "refactor(app): keep one shell DPI source"
```

Expected: 第一条扫描无结构/clone 残留；测试和 check PASS。

### Task 14: Settings 残留审计与总验收

**Files:**
- Modify only if scan finds an omitted production call; stop and create a new atomic task before editing more than 3 files.

- [ ] **Step 1: 列出全部剩余 Settings::new**

```bash
rg -n "Settings::new\(\)" crates/app/src -g '*.rs'
```

Expected: 生产路径只保留 `app_init.rs::settings_from_persisted` 的根构造；其余命中均位于 `#[cfg(test)]` 测试模块。

- [ ] **Step 2: 验证无 clone/drop 模式**

```bash
rg -n "self\.settings\.clone\(\)|let mut s = Settings::new\(\).*apply_scale" \
  crates/app/src -g '*.rs'
```

Expected: 无输出。

- [ ] **Step 3: 运行 Settings 与 DPI 相关测试**

```bash
cargo test -p edit-plus-ui --lib settings::tests::
cargo test -p edit-plus-app --lib app_scroll::tests::
cargo test -p edit-plus-app --lib app_reshape::zoom_tests::
cargo test -p edit-plus-app --lib app_window::
cargo test -p edit-plus-app --lib workspace::tests::
cargo test -p edit-plus-app --lib md_preview::heading_tests::
```

Expected: 全部 PASS。

- [ ] **Step 4: 运行 crate 全量验证**

```bash
cargo check -p edit-plus-app
cargo test -p edit-plus-app --lib
cargo test -p edit-plus-ui --lib
```

Expected: app check、app lib tests、ui lib tests PASS。记录现有 warning 数量，不在本轮扩大范围清理。

- [ ] **Step 5: 记录 workspace gate 的既有阻塞**

```bash
cargo check --workspace --all-targets
```

Expected: 如果仍因 `crates/core/src/buffer/text_buffer_tests.rs` 的重复测试名失败，在交付说明中标为本轮开始前已存在且不属于 Settings/DPI 修改；不得声称 workspace gate 全绿。

- [ ] **Step 6: 对扫描结果做停止条件判断**

若 Step 1/2 找到新的生产路径遗漏，停止本任务，把每个遗漏按最多 3 个文件拆成新的 TDD 任务并追加到本计划；不得在总验收阶段直接顺手编辑。若没有遗漏，不创建空提交。
