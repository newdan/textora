# Logical Settings / Physical UiMetrics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** 将 Settings 固定为 DPI 无关的逻辑配置，将所有布局、绘制、命中测试和 reshape 路径统一改为消费由 Settings 与 App.scale_factor 纯派生的物理 UiMetrics。

**Architecture:** 先引入可同时服务旧物理 Settings 和新逻辑 Settings 的短期兼容入口，再逐条迁移 App、render pipeline、widgets 与测试。所有消费者完成迁移后，在一个原子提交中切换 App facade 的语义，最后删除 Settings 上的 DPI 状态与旧构造入口；因此任一中间提交都不会混用逻辑和物理单位。

**Tech Stack:** Rust、winit、wgpu、现有 Settings/UiMetrics/App/UiShell/SidebarWidget、app 与 ui 单元测试。

---

**设计依据：** docs/superpowers/specs/2026-06-20-logical-settings-physical-metrics-design.md

**前置条件：** 先完整执行 docs/plans-settings-dpi-remediation.md。本文假定生产路径中的 Settings::new() 回读、Tabs/Markdown/Workspace viewport 和 Zoom 行为回归已经按该计划修复。

**共同验收约束：**

- 每个任务最多修改 3 个文件。
- 行为变化先写失败测试，再写最小实现。
- 机械签名迁移也必须先完成调用点清单，并在提交前运行受影响 crate 测试。
- 每次提交前至少运行 cargo check -p edit-plus-app；UI 任务还要运行 cargo test -p edit-plus-ui --lib。
- 不在本计划中处理 AppEffect、公共 API 收缩、ThemeRegistry、warning 或 core 重复测试名。
- 兼容 API 只能在 Task 2 至 Task 17 存在；Task 18 必须删除。

**Phase 2 范围闭环：** 本计划覆盖逻辑 Settings、物理 UiMetrics、DPI 生命周期、所有已知物理尺寸消费者、Sidebar 行为输入拆分、缓存失效、持久化往返和源码门禁。AppEffect、公共 API 收缩、ThemeRegistry 与 warning 治理明确留在后续边界重构计划中，不作为本阶段完成条件。

## 文件职责映射

- crates/ui/src/settings.rs：定义逻辑 Settings、纯派生 UiMetrics 及 DPI 规范化。
- crates/app/src/app.rs：唯一 App 级 metrics facade、scale factor 更新和逻辑字号 mutation facade。
- crates/app/src/app_init.rs：持久化逻辑设置装配；不再构造物理 Settings。
- crates/app/src/app_window.rs：窗口 scale factor 采集、TextState 与 reshape worker 的物理初始化。
- crates/app/src/app_lifecycle.rs：ScaleFactorChanged 的可逆 metrics 更新和缓存失效。
- crates/app/src/app_reshape.rs：逻辑 zoom 步长和物理 reshape request。
- crates/ui/src/decorations.rs：只消费物理 metrics 的 cursor/selection/search 装饰。
- crates/app/src/render_pipeline.rs：只消费物理 metrics 的 shaping、viewport 与 gutter 计算。
- crates/app/src/mouse.rs：只消费物理 line height 的命中测试。
- crates/app/src/events.rs：从 App facade 获取同一事件快照的 metrics。
- crates/app/src/app_renderer.rs：每帧派生一次 metrics，并传给 render/Markdown/decorations。
- crates/app/src/app_scroll.rs：使用物理 metrics 处理滚动、TOC 和 viewport。
- crates/app/src/app_dispatch.rs：使用物理 metrics 处理 preview 与 sidebar actions。
- crates/app/src/app_search.rs：搜索跳转后使用物理行高保持光标可见。
- crates/app/src/dispatch/editor.rs：编辑完成后的 viewport/cursor follow-up 使用物理行高。
- crates/app/src/dispatch/mouse.rs：鼠标编辑 follow-up 使用物理行高。
- crates/ui/src/widgets/sidebar/types.rs：定义独立 SidebarSettingsInput。
- crates/ui/src/widgets/sidebar/menu.rs：用行为输入构造设置菜单。
- crates/ui/src/widgets/sidebar/state.rs：保存/消费 SidebarSettingsInput，不回读 UiMetrics 行为字段。
- crates/ui/src/widgets/sidebar/mod.rs：SidebarWidget 同时接收布局 metrics 和行为 input。
- crates/app/src/ui_shell.rs：ShellInputs 携带 metrics 与 SidebarSettingsInput，删除重复 dpi 字段。
- crates/app/src/settings_boundary_tests.rs：覆盖全部物理消费者和旧 DPI API 的源码门禁。

## 迁移不变量

1. Task 1 至 Task 16 中，Settings 仍保存物理尺寸；App::ui_metrics() 必须调用 UiMetrics::from_physical_settings()。
2. Task 17 是唯一语义切换提交：从该提交开始 Settings 保存逻辑尺寸，App::ui_metrics() 调用 UiMetrics::from_settings(settings, scale_factor)。
3. 任何绘制、布局、命中测试、viewport、reshape request 都只能读取 UiMetrics 的尺寸字段。
4. 任何持久化和 zoom mutation 都只能读写 Settings 的逻辑字段。
5. 同一事件或同一帧只派生一次 UiMetrics；向下层按值或引用传递，不在子函数重新读取 App。

### Task 1: 新增纯 UiMetrics 派生 API 和兼容构造器

**Files:**
- Modify/Test: crates/ui/src/settings.rs

- [ ] **Step 1: 写纯派生和无效 DPI 失败测试**

在 settings.rs 测试模块增加：

~~~rust
#[test]
fn ui_metrics_scale_logical_dimensions_exactly_once() {
    let mut settings = Settings::new();
    settings.font_size = 10.0;
    settings.line_height = 16.0;
    settings.status_bar_height = 20.0;
    settings.gutter_padding = 8.0;
    settings.toc_width = 200.0;

    let metrics = UiMetrics::from_settings(&settings, 2.0);

    assert_eq!(metrics.dpi, 2.0);
    assert_eq!(metrics.font_size, 20.0);
    assert_eq!(metrics.line_height, 32.0);
    assert_eq!(metrics.status_bar_height, 40.0);
    assert_eq!(metrics.gutter_padding, 16.0);
    assert_eq!(metrics.toc_width, 400.0);
    assert_eq!(metrics.content_left_margin, 64.0);
    assert_eq!(
        metrics.scrollbar_reserve,
        crate::widgets::scrollbar::SCROLLBAR_RESERVE_PX * 2.0
    );
}

#[test]
fn ui_metrics_invalid_dpi_falls_back_to_one() {
    let settings = Settings::new();
    for dpi in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let metrics = UiMetrics::from_settings(&settings, dpi);
        assert_eq!(metrics.dpi, 1.0);
        assert_eq!(metrics.font_size, settings.font_size);
        assert_eq!(metrics.line_height, settings.line_height);
    }
}

#[test]
fn ui_metrics_derivation_is_repeatable() {
    let settings = Settings::new();
    assert_eq!(
        UiMetrics::from_settings(&settings, 1.75),
        UiMetrics::from_settings(&settings, 1.75)
    );
}
~~~

- [ ] **Step 2: 运行测试并确认缺少新 API**

~~~bash
cargo test -p edit-plus-ui --lib settings::tests::ui_metrics_scale_logical_dimensions_exactly_once -- --exact
~~~

Expected: FAIL，UiMetrics::from_settings 不存在或新字段不存在。

- [ ] **Step 3: 扩展 UiMetrics 并实现两个明确构造入口**

在 UiMetrics 增加 toc_width、content_left_margin、scrollbar_reserve。实现：

~~~rust
impl UiMetrics {
    fn normalize_dpi(dpi: f32) -> f32 {
        if dpi.is_finite() && dpi > 0.0 { dpi } else { 1.0 }
    }

    pub fn from_settings(settings: &Settings, dpi: f32) -> Self {
        let dpi = Self::normalize_dpi(dpi);
        Self {
            dpi,
            font_size: settings.font_size * dpi,
            line_height: settings.line_height * dpi,
            status_bar_height: settings.status_bar_height * dpi,
            gutter_padding: settings.gutter_padding * dpi,
            toc_width: settings.toc_width * dpi,
            content_left_margin: 32.0 * dpi,
            scrollbar_reserve: crate::widgets::scrollbar::SCROLLBAR_RESERVE_PX * dpi,
            show_line_numbers: settings.show_line_numbers,
            show_status_bar: settings.show_status_bar,
            word_wrap: settings.word_wrap,
            theme_mode: settings.theme_mode,
        }
    }

    #[doc(hidden)]
    pub fn from_physical_settings(settings: &Settings) -> Self {
        Self {
            dpi: Self::normalize_dpi(settings.dpi_scale),
            font_size: settings.font_size,
            line_height: settings.line_height,
            status_bar_height: settings.status_bar_height,
            gutter_padding: settings.gutter_padding,
            toc_width: settings.toc_width,
            content_left_margin: settings.content_left_margin(),
            scrollbar_reserve: settings.scrollbar_reserve(),
            show_line_numbers: settings.show_line_numbers,
            show_status_bar: settings.show_status_bar,
            word_wrap: settings.word_wrap,
            theme_mode: settings.theme_mode,
        }
    }
}

impl From<&Settings> for UiMetrics {
    fn from(settings: &Settings) -> Self {
        Self::from_physical_settings(settings)
    }
}
~~~

此任务不删除 UiMetrics 的行为字段；Sidebar 迁移前仍需兼容。

- [ ] **Step 4: 运行 UI 测试和编译检查**

~~~bash
cargo test -p edit-plus-ui --lib settings::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/ui/src/settings.rs
git commit -m "refactor(ui): add pure metrics derivation"
~~~

### Task 2: 建立 App 单位兼容 facade

**Files:**
- Modify: crates/app/src/app.rs
- Modify/Test: crates/app/src/app_tests.rs
- Modify/Test: crates/app/src/app_reshape.rs

- [ ] **Step 1: 写 facade 与 zoom 失败测试**

在 app_tests.rs 增加：

~~~rust
#[test]
fn compatibility_metrics_do_not_double_scale_physical_settings() {
    let mut app = App::new(None);
    app.settings.apply_scale(2.0);

    let metrics = app.ui_metrics();

    assert_eq!(metrics.dpi, 2.0);
    assert_eq!(metrics.font_size, app.settings.font_size);
    assert_eq!(metrics.line_height, app.settings.line_height);
}

#[test]
fn persisted_font_size_is_logical_during_compatibility_stage() {
    let mut app = App::new(None);
    app.settings.apply_scale(2.0);
    assert_eq!(app.persisted_font_size(), 15.0);
}
~~~

在 app_reshape.rs 的测试模块增加：

~~~rust
#[test]
fn logical_zoom_step_is_one_at_two_x_dpi() {
    let mut app = App::new(None);
    app.update_scale_factor(2.0);
    let before = app.persisted_font_size();

    app.apply_zoom(before + 1.0);

    assert_eq!(app.persisted_font_size(), before + 1.0);
}
~~~

- [ ] **Step 2: 运行测试并确认 facade 不存在**

~~~bash
cargo test -p edit-plus-app --lib app::app_tests::compatibility_metrics_do_not_double_scale_physical_settings -- --exact
cargo test -p edit-plus-app --lib app_reshape::tests::logical_zoom_step_is_one_at_two_x_dpi -- --exact
~~~

Expected: FAIL，ui_metrics/update_scale_factor/persisted_font_size 尚不存在。

- [ ] **Step 3: 在 App 增加兼容 facade**

~~~rust
pub(crate) fn ui_metrics(&self) -> ui::settings::UiMetrics {
    ui::settings::UiMetrics::from_physical_settings(&self.settings)
}

pub(crate) fn update_scale_factor(&mut self, scale_factor: f64) {
    self.scale_factor = scale_factor;
    self.settings.apply_scale(scale_factor);
}

pub(crate) fn persisted_font_size(&self) -> f32 {
    self.settings.logical_font_size()
}

pub(crate) fn set_logical_font_size(&mut self, logical_size: f32) {
    let physical_size = logical_size * self.settings.dpi_scale;
    self.settings.set_font_size(physical_size);
}
~~~

将 app_reshape.rs 的 zoom mutation 收口到：

~~~rust
let logical_font_size = logical_font_size.clamp(6.0, 72.0);
self.set_logical_font_size(logical_font_size);
let metrics = self.ui_metrics();
~~~

reshape request、display map 与 TextState 更新全部使用 metrics.font_size 和 metrics.line_height。

- [ ] **Step 4: 运行定向测试和编译检查**

~~~bash
cargo test -p edit-plus-app --lib app::app_tests::compatibility_metrics_do_not_double_scale_physical_settings -- --exact
cargo test -p edit-plus-app --lib app_reshape::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/app.rs crates/app/src/app_tests.rs crates/app/src/app_reshape.rs
git commit -m "refactor(app): centralize settings unit conversion"
~~~

### Task 3: 将 UI decorations 改为只接收物理 UiMetrics

**Files:**
- Modify/Test: crates/ui/src/decorations.rs
- Modify: crates/app/src/app_renderer.rs

- [ ] **Step 1: 写 2x cursor 尺寸失败测试**

将 decorations.rs 中 cursor 测试改为显式构造：

~~~rust
let settings = Settings::new();
let metrics = UiMetrics::from_settings(&settings, 2.0);
let vertices = cursor_vertices(
    &theme,
    Some(0),
    0.0,
    40.0,
    Instant::now(),
    &metrics,
    800.0,
    600.0,
    0.0,
    None,
);
let cursor_width_px = (vertices[1].position[0] - vertices[0].position[0]) * 800.0 / 2.0;
assert!((cursor_width_px - 4.0).abs() < 0.01);
~~~

- [ ] **Step 2: 运行测试并确认签名或宽度断言失败**

~~~bash
cargo test -p edit-plus-ui --lib decorations::tests -- --nocapture
~~~

Expected: FAIL；函数仍要求 &Settings，或 cursor width 未从 metrics.dpi 派生。

- [ ] **Step 3: 替换 decorations 参数与字段读取**

将 selection_vertices、cursor_vertices、search_match_vertices 的 settings 参数统一替换为：

~~~rust
metrics: &crate::settings::UiMetrics
~~~

逐项替换：

~~~rust
settings.line_height  => metrics.line_height
settings.font_size    => metrics.font_size
settings.dpi_scale    => metrics.dpi
~~~

在 app_renderer.rs 帧入口添加一次：

~~~rust
let metrics = self.ui_metrics();
~~~

并把所有 decoration 调用的 &self.settings 改为 &metrics。

- [ ] **Step 4: 运行 UI 测试和 app 编译**

~~~bash
cargo test -p edit-plus-ui --lib decorations::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/ui/src/decorations.rs crates/app/src/app_renderer.rs
git commit -m "refactor(ui): render decorations from metrics"
~~~

### Task 4: 将 render pipeline 改为只接收物理 UiMetrics

**Files:**
- Modify/Test: crates/app/src/render_pipeline.rs
- Modify/Test: crates/app/src/render_pipeline_tests.rs
- Modify: crates/app/src/app_renderer.rs

- [ ] **Step 1: 写 viewport 宽度失败测试**

在 render_pipeline_tests.rs 增加一个纯辅助函数测试，并先提取目标公式：

~~~rust
#[test]
fn render_viewport_width_uses_physical_scrollbar_reserve() {
    let settings = ui::settings::Settings::new();
    let metrics = ui::settings::UiMetrics::from_settings(&settings, 2.0);
    assert_eq!(
        render_viewport_width(1000.0, 64.0, &metrics),
        1000.0 - 64.0 - metrics.scrollbar_reserve
    );
}
~~~

- [ ] **Step 2: 运行测试并确认辅助函数不存在**

~~~bash
cargo test -p edit-plus-app --lib render_pipeline::tests::render_viewport_width_uses_physical_scrollbar_reserve -- --exact
~~~

Expected: FAIL，render_viewport_width 不存在。

- [ ] **Step 3: 改造 pipeline 签名**

增加：

~~~rust
fn render_viewport_width(
    screen_w: f32,
    left_margin: f32,
    metrics: &ui::settings::UiMetrics,
) -> f32 {
    screen_w - left_margin - metrics.scrollbar_reserve
}
~~~

将 render_line_number_placeholder、preedit_text_vertices 的 Settings 参数改为：

~~~rust
metrics: &ui::settings::UiMetrics
~~~

shape_visible_lines 的开头改为：

~~~rust
pub(crate) fn shape_visible_lines(
    metrics: &ui::settings::UiMetrics,
    min_punctuation_width_ratio: f32,
    ctx: &ui::gutter::RenderContext,
    dv: &mut DocumentView,
    text: &mut TextState,
    gpu: &GpuState,
    advance_cache: &mut Vec<AdvanceCacheEntry>,
    cluster_pool: &mut Vec<Vec<(usize, f32)>>,
    first_line: &mut LineCache,
    last_line: &mut LineCache,
    tree_dirty: &mut bool,
) -> Vec<GlyphVertex>
~~~

字段映射固定为：

~~~rust
settings.font_size          => metrics.font_size
settings.line_height         => metrics.line_height
settings.gutter_padding      => metrics.gutter_padding
settings.show_line_numbers   => metrics.show_line_numbers
settings.scrollbar_reserve() => metrics.scrollbar_reserve
settings.min_punctuation_width_ratio => min_punctuation_width_ratio
~~~

app_renderer.rs 继续复用 Task 3 的单帧 metrics，并传入 &metrics 与 self.settings.min_punctuation_width_ratio。

- [ ] **Step 4: 运行定向测试、app 测试和编译**

~~~bash
cargo test -p edit-plus-app --lib render_pipeline::tests::render_viewport_width_uses_physical_scrollbar_reserve -- --exact
cargo test -p edit-plus-app --lib render_pipeline::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/render_pipeline.rs crates/app/src/render_pipeline_tests.rs crates/app/src/app_renderer.rs
git commit -m "refactor(app): drive render pipeline with metrics"
~~~

### Task 5: 将 editor hit-test 改为物理 metrics

**Files:**
- Modify/Test: crates/app/src/mouse.rs
- Modify/Test: crates/app/src/events.rs

- [ ] **Step 1: 写 2x 行高命中测试**

在 mouse.rs 测试模块增加：

~~~rust
#[test]
fn hit_test_uses_physical_line_height() {
    let dv = make_dv("abcdefghij");
    let cache = vec![
        AdvanceCacheEntry {
            doc_line: 0,
            vl_byte_start: 0,
            clusters: vec![(5, 130.0)],
        },
        AdvanceCacheEntry {
            doc_line: 0,
            vl_byte_start: 5,
            clusters: vec![(5, 130.0)],
        },
    ];
    let settings = Settings::new();
    let metrics = UiMetrics::from_settings(&settings, 2.0);

    let hit = hit_test(
        40.0,
        metrics.line_height + 1.0,
        &dv,
        &cache,
        &metrics,
        32.0,
        0.0,
    )
    .unwrap();

    assert_eq!(hit.2, 1);
}
~~~

- [ ] **Step 2: 运行测试并确认签名失败**

~~~bash
cargo test -p edit-plus-app --lib mouse::tests::hit_test_uses_physical_line_height -- --exact
~~~

Expected: FAIL，hit_test 仍要求 &Settings。

- [ ] **Step 3: 修改 hit-test 与事件调用点**

mouse.rs：

~~~rust
pub(crate) fn hit_test(
    px: f32,
    py: f32,
    dv: &DocumentView,
    advance_cache: &[AdvanceCacheEntry],
    metrics: &UiMetrics,
    left_margin: f32,
    tab_bar_height: f32,
) -> Option<(usize, usize, usize)> {
    let sub_line_offset =
        dv.display.viewport.sub_line_pixel_offset(metrics.line_height);
    let adjusted_py = py - tab_bar_height - sub_line_offset;
    if adjusted_py < 0.0 {
        return None;
    }
    let vis_line = (adjusted_py / metrics.line_height) as usize;
    if vis_line >= advance_cache.len() {
        return None;
    }
    let entry = &advance_cache[vis_line];
    // 从这里继续执行现有 cluster snap 和 document offset 映射。
}
~~~

events.rs 的每个 mouse handler 在借用 active document 前构造一次：

~~~rust
let metrics = app.ui_metrics();
~~~

mouse_hit_test 调用统一传 &metrics；EventCtx 的 dpi 改为 metrics.dpi。

- [ ] **Step 4: 运行 mouse/events 测试和编译**

~~~bash
cargo test -p edit-plus-app --lib mouse::tests -- --nocapture
cargo test -p edit-plus-app --lib events::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/mouse.rs crates/app/src/events.rs
git commit -m "refactor(app): hit test with physical metrics"
~~~

### Task 6: 将 App 几何、滚动和 dispatch 改为 metrics

**Files:**
- Modify/Test: crates/app/src/app.rs
- Modify/Test: crates/app/src/app_scroll.rs
- Modify/Test: crates/app/src/app_dispatch.rs

- [ ] **Step 1: 写 2x 几何与滚动哨兵测试**

在 app.rs 增加内联 geometry_metrics_tests 模块：

~~~rust
#[cfg(test)]
mod geometry_metrics_tests {
use super::App;
use crate::document_view::DocumentView;
use crate::view::View;

#[test]
fn app_geometry_uses_metrics_snapshot() {
    let mut app = App::new(None);
    app.workspace.push_view_for_test(View::Editor(DocumentView::new(
        vec!["first".into()],
        80,
        10.0,
    )));
    app.workspace.push_view_for_test(View::Editor(DocumentView::new(
        vec!["second".into()],
        80,
        10.0,
    )));
    app.workspace.switch_to(0);
    app.update_scale_factor(2.0);
    app.settings.view_mode = ui::view_mode::ViewMode::Tabs;

    let metrics = app.ui_metrics();
    assert_eq!(app.content_left_margin(), metrics.content_left_margin);
    assert_eq!(app.current_tab_bar_height(), ui::tab_bar::tab_bar_height(metrics.dpi));
}
}
~~~

在 app_scroll.rs 增加：

~~~rust
#[test]
fn line_scroll_uses_app_metrics_line_height() {
    let mut app = App::new(None);
    let dv = DocumentView::new(
        (0..100).map(|i| format!("line {i}")).collect(),
        80,
        10.0,
    );
    app.workspace
        .push_view_for_test(crate::view::View::Editor(dv));
    app.workspace.switch_to(0);
    app.workspace
        .active_doc_mut()
        .unwrap()
        .display
        .display_map
        .set_entries(
            (0..100)
                .map(|i| crate::snap_tree::DisplayLineEntry::placeholder(i, 10, 0, 1))
                .collect(),
        );
    app.update_scale_factor(2.0);
    let line_height = app.ui_metrics().line_height;
    app.handle_scroll(MouseScrollDelta::PixelDelta(
        winit::dpi::PhysicalPosition::new(0.0, -(line_height as f64)),
    ));
    let viewport = &app.workspace.active_doc().unwrap().display.viewport;
    assert!((viewport.scroll_top - 1.0).abs() < 0.01);
}
~~~

- [ ] **Step 2: 运行测试并确认旧字段仍被直接读取**

~~~bash
cargo test -p edit-plus-app --lib app_scroll::tests::line_scroll_uses_app_metrics_line_height -- --exact
~~~

Expected: FAIL，滚动仍读取 Settings 尺寸或默认对象。

- [ ] **Step 3: 在三个文件内统一使用快照**

app.rs 的 geometry 方法开头使用：

~~~rust
let metrics = self.ui_metrics();
~~~

并按以下映射替换：dpi_scale → metrics.dpi，content_left_margin() → metrics.content_left_margin，status_bar_height → metrics.status_bar_height。

app_scroll.rs 每个 public App handler 在可变借用 Workspace 前添加：

~~~rust
let metrics = self.ui_metrics();
let line_height = metrics.line_height;
let dpi = metrics.dpi;
let toc_width = metrics.toc_width;
~~~

app_dispatch.rs 的 preview offset、sidebar width 和 settings menu layout 使用同一 handler 的 metrics，不构造第二份快照。

- [ ] **Step 4: 运行相关测试、静态扫描和编译**

~~~bash
cargo test -p edit-plus-app --lib app_scroll::tests -- --nocapture
cargo test -p edit-plus-app --lib app::app_tests -- --nocapture
rg -n "settings\.(dpi_scale|font_size|line_height|status_bar_height|gutter_padding|toc_width)" crates/app/src/app.rs crates/app/src/app_scroll.rs crates/app/src/app_dispatch.rs
cargo check -p edit-plus-app
~~~

Expected: 测试 PASS；扫描仅允许逻辑行为或将在 Task 17 切换的 facade 内命中。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/app.rs crates/app/src/app_scroll.rs crates/app/src/app_dispatch.rs
git commit -m "refactor(app): derive interaction geometry from metrics"
~~~

### Task 7: 将 Markdown 和 renderer 剩余尺寸改为 metrics

**Files:**
- Modify/Test: crates/app/src/md_preview.rs
- Modify/Test: crates/app/src/app_renderer.rs
- Modify: crates/app/src/events.rs

- [ ] **Step 1: 写 Markdown 物理字号失败测试**

在 md_preview.rs 测试模块增加：

~~~rust
#[test]
fn markdown_render_settings_take_physical_metrics() {
    let settings = ui::settings::Settings::new();
    let metrics = ui::settings::UiMetrics::from_settings(&settings, 2.0);
    let input = MarkdownRenderSettings::from_metrics(&settings, &metrics);

    assert_eq!(input.font_size, metrics.font_size);
    assert_eq!(input.line_height, metrics.line_height);
    assert_eq!(input.toc_max_depth, settings.toc_max_depth);
}
~~~

- [ ] **Step 2: 运行测试并确认构造入口不存在**

~~~bash
cargo test -p edit-plus-app --lib md_preview::tests::markdown_render_settings_take_physical_metrics -- --exact
~~~

Expected: FAIL，from_metrics 不存在。

- [ ] **Step 3: 分离 Markdown 行为和物理尺寸**

实现：

~~~rust
impl MarkdownRenderSettings {
    pub(crate) fn from_metrics(
        settings: &ui::settings::Settings,
        metrics: &ui::settings::UiMetrics,
    ) -> Self {
        Self {
            font_size: metrics.font_size,
            line_height: metrics.line_height,
            toc_max_depth: settings.toc_max_depth,
        }
    }
}
~~~

app_renderer.rs 的帧快照同时传给 Markdown、gutter、viewport 和 decorations。events.rs 中 preview hit-test 也使用 handler 开头构造的 metrics。

- [ ] **Step 4: 运行 Markdown/renderer 相关测试和编译**

~~~bash
cargo test -p edit-plus-app --lib md_preview::tests -- --nocapture
cargo test -p edit-plus-app --lib events::tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/md_preview.rs crates/app/src/app_renderer.rs crates/app/src/events.rs
git commit -m "refactor(app): render markdown from metrics"
~~~

### Task 8: 收口窗口 DPI 生命周期并延后 worker 初始化

**Files:**
- Modify/Test: crates/app/src/app_window.rs
- Modify/Test: crates/app/src/app_lifecycle.rs
- Modify: crates/app/src/app_init.rs

- [ ] **Step 1: 写 DPI 往返与持久化失败测试**

在 app_lifecycle.rs 测试模块增加可直接调用的内部 helper 测试；先把 ScaleFactorChanged 分支核心提取为 App 方法 handle_scale_factor_changed：

~~~rust
#[test]
fn scale_factor_round_trip_preserves_logical_persistence_value() {
    let mut app = App::new(None);
    let logical = app.persisted_font_size();

    app.handle_scale_factor_changed(2.0);
    assert_eq!(app.persisted_font_size(), logical);
    app.handle_scale_factor_changed(1.0);
    assert_eq!(app.persisted_font_size(), logical);
}
~~~

在 app_window.rs 增加内联 dpi_initialization_tests 模块：

~~~rust
#[cfg(test)]
mod dpi_initialization_tests {
use super::App;

#[test]
fn window_initialization_uses_one_metrics_snapshot_for_text_and_worker() {
    let mut app = App::new(None);
    app.update_scale_factor(2.0);
    let metrics = app.ui_metrics();
    let initial = App::initial_window_metrics(metrics);
    assert_eq!(initial.font_size, metrics.font_size);
    assert_eq!(initial.line_height, metrics.line_height);
    assert_eq!(initial.dpi, metrics.dpi);
}
}
~~~

再在 app_lifecycle.rs 增加实际 cache/shell 失效测试：

~~~rust
#[test]
fn scale_factor_change_invalidates_all_physical_caches_and_shell_layout() {
    use crate::render_cache::CachedLine;
    use ui::render_geom::AdvanceCacheEntry;

    let mut app = App::new(None);
    app.workspace.push_view_for_test(crate::view::View::Editor(
        crate::document_view::DocumentView::new(vec!["line".into()], 40, 40.0),
    ));
    app.workspace.view_mut(0).unwrap().doc_mut().display.display_map.set_entries(vec![
        crate::snap_tree::DisplayLineEntry::placeholder(0, 4, 0, 1),
        crate::snap_tree::DisplayLineEntry::placeholder(1, 0, 0, 1),
    ]);
    app.workspace.view_mut(0).unwrap().doc_mut().display.render_cache.insert(
        0,
        CachedLine {
            instances: Vec::new(),
            line_number_glyphs: Vec::new(),
            atlas_generation: 1,
            visual_line_count: 1,
            content_hash: 1,
            visual_lines: Vec::new(),
            visual_line_instance_starts: Vec::new(),
            cluster_data: Vec::new(),
            subset_start: 0,
        },
    );
    app.frame_cache.advance_cache.push(AdvanceCacheEntry {
        doc_line: 0,
        vl_byte_start: 0,
        clusters: Vec::new(),
    });
    app.frame_cache.cluster_pool.push(Vec::new());
    app.ui_shell.dock_dirty = false;
    let generation = app.reshape_generation;

    app.handle_scale_factor_changed(2.0);

    assert!(app.workspace.view(0).unwrap().doc().display.render_cache.is_empty());
    assert!(app.frame_cache.advance_cache.is_empty());
    assert!(app.frame_cache.cluster_pool.is_empty());
    assert!(app.reshape_generation > generation);
    assert!(app.ui_shell.dock_dirty);
    assert_eq!(
        app.workspace.view(0).unwrap().doc().display.display_map.line_count(),
        1,
    );
}
~~~

`InitialWindowMetrics` 是从单个 UiMetrics 按值生成的不可变启动快照，TextState、display map、viewport、UiShell 和 worker 都只能读取它或原始 metrics。

- [ ] **Step 2: 运行测试并确认 lifecycle/helper 尚未收口**

~~~bash
cargo test -p edit-plus-app --lib app_lifecycle::tests::scale_factor_round_trip_preserves_logical_persistence_value -- --exact
cargo test -p edit-plus-app --lib app_window::dpi_initialization_tests::window_initialization_uses_one_metrics_snapshot_for_text_and_worker -- --exact
~~~

Expected: FAIL，helper 不存在。

- [ ] **Step 3: 迁移窗口与生命周期到 App facade**

app_window.rs：

~~~rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct InitialWindowMetrics {
    pub(crate) dpi: f32,
    pub(crate) font_size: f32,
    pub(crate) line_height: f32,
}

self.update_scale_factor(window.scale_factor());
let metrics = self.ui_metrics();
let initial = Self::initial_window_metrics(metrics);
~~~

并实现：

~~~rust
pub(crate) fn initial_window_metrics(metrics: ui::settings::UiMetrics) -> InitialWindowMetrics {
    InitialWindowMetrics {
        dpi: metrics.dpi,
        font_size: metrics.font_size,
        line_height: metrics.line_height,
    }
}
~~~

TextState、display map、viewport、UiShell 和 ReshapeWorker 初始化全部复用 `metrics`/`initial`，init_window 在更新 scale factor 后不得再次调用 `self.ui_metrics()`。save_window_geometry 使用 self.persisted_font_size()，不直接调用 Settings::logical_font_size()。

sidebar 宽度持久化明确保持逻辑单位：

~~~rust
persisted.sidebar_width =
    self.ui_shell.sidebar_width() / metrics.dpi;
~~~

窗口位置和宽高继续沿用现有物理几何保存契约。

app_init.rs 删除提前创建 ReshapeWorker 的代码，让字段保持 None；app_window.rs 在 scale factor 已知后创建 worker：

~~~rust
self.reshape_worker = Some(crate::reshape_worker::ReshapeWorker::spawn(
    self.shared_font_system
        .clone()
        .expect("FontSystem not initialized"),
    metrics.font_size,
    self.settings.font_family.clone(),
));
~~~

app_lifecycle.rs：

~~~rust
pub(crate) fn handle_scale_factor_changed(&mut self, scale_factor: f64) {
    let old_metrics = self.ui_metrics();
    self.update_scale_factor(scale_factor);
    let new_metrics = self.ui_metrics();
    let ratio = new_metrics.dpi / old_metrics.dpi;
    self.ui_shell.sidebar_cfg_mut().width *= ratio;
    self.ui_shell.sidebar_clamp_width(new_metrics.dpi);
    for index in 0..self.workspace.len() {
        self.workspace
            .view_mut(index)
            .unwrap()
            .doc_mut()
            .display
            .render_cache
            .invalidate_all();
    }
    self.frame_cache.advance_cache.clear();
    self.frame_cache.cluster_pool.clear();
    self.ui_shell.dock_dirty = true;
    if self.workspace.active_index() < self.workspace.len() {
        self.init_display_map(self.workspace.active_index());
        self.invalidate_reshape();
    }
    self.needs_redraw = true;
}
~~~

ScaleFactorChanged 分支只调用 handle_scale_factor_changed(scale_factor)，不得再直接修改 Settings。

- [ ] **Step 4: 运行生命周期测试和编译**

~~~bash
cargo test -p edit-plus-app --lib app_lifecycle::tests -- --nocapture
cargo test -p edit-plus-app --lib app_window::dpi_initialization_tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/app_window.rs crates/app/src/app_lifecycle.rs crates/app/src/app_init.rs
git commit -m "refactor(app): centralize window scale lifecycle"
~~~

### Task 9: 定义独立 SidebarSettingsInput

**Files:**
- Modify/Test: crates/ui/src/widgets/sidebar/types.rs
- Modify/Test: crates/ui/src/widgets/sidebar/menu.rs
- Modify/Test: crates/app/src/app_dispatch.rs

- [ ] **Step 1: 写行为输入独立性失败测试**

在 types.rs 测试模块增加：

~~~rust
#[test]
fn sidebar_settings_input_copies_behavior_only() {
    let mut settings = crate::settings::Settings::new();
    settings.show_line_numbers = false;
    settings.word_wrap = false;
    settings.show_status_bar = true;
    settings.theme_mode = crate::settings::ThemeMode::Dark;
    settings.view_mode = crate::view_mode::ViewMode::Tabs;

    let input = SidebarSettingsInput::from(&settings);

    assert!(!input.show_line_numbers);
    assert!(!input.word_wrap);
    assert!(input.show_status_bar);
    assert_eq!(input.theme_mode, crate::settings::ThemeMode::Dark);
    assert_eq!(input.view_mode, crate::view_mode::ViewMode::Tabs);
}
~~~

在 menu.rs 测试模块增加：

~~~rust
#[test]
fn menu_geometry_uses_metrics_and_checks_use_behavior_input() {
    let settings = crate::settings::Settings::new();
    let metrics = crate::settings::UiMetrics::from_settings(&settings, 2.0);
    let input = SidebarSettingsInput {
        show_line_numbers: false,
        word_wrap: false,
        show_status_bar: true,
        theme_mode: crate::settings::ThemeMode::Dark,
        view_mode: crate::view_mode::ViewMode::Tabs,
    };
    let menu = build_settings_menu(None, &input, 800.0, 600.0, &metrics)
        .expect("menu");

    assert_eq!(menu.menu_rect.w, 400.0);
    assert!(!menu.items[0].is_active);
    assert!(!menu.items[1].is_active);
    assert!(menu.items[2].is_active);
    assert!(menu.items[5].is_active);
    assert!(menu.items[9].is_active);
}
~~~

- [ ] **Step 2: 运行测试并确认类型不存在**

~~~bash
cargo test -p edit-plus-ui --lib widgets::sidebar::types::tests::sidebar_settings_input_copies_behavior_only -- --exact
~~~

Expected: FAIL，SidebarSettingsInput 不存在。

- [ ] **Step 3: 实现行为输入并改造菜单 builder**

types.rs：

~~~rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarSettingsInput {
    pub show_line_numbers: bool,
    pub word_wrap: bool,
    pub show_status_bar: bool,
    pub theme_mode: crate::settings::ThemeMode,
    pub view_mode: crate::view_mode::ViewMode,
}

impl From<&crate::settings::Settings> for SidebarSettingsInput {
    fn from(settings: &crate::settings::Settings) -> Self {
        Self {
            show_line_numbers: settings.show_line_numbers,
            word_wrap: settings.word_wrap,
            show_status_bar: settings.show_status_bar,
            theme_mode: settings.theme_mode,
            view_mode: settings.view_mode,
        }
    }
}

impl Default for SidebarSettingsInput {
    fn default() -> Self {
        Self {
            show_line_numbers: true,
            word_wrap: true,
            show_status_bar: false,
            theme_mode: crate::settings::ThemeMode::default(),
            view_mode: crate::view_mode::ViewMode::default(),
        }
    }
}
~~~

menu.rs 的 build_settings_menu 签名改为同时接收：

~~~rust
use super::types::SidebarSettingsInput;

pub fn build_settings_menu(
    settings_btn_rect: Option<crate::core::Rect>,
    input: &SidebarSettingsInput,
    screen_w: f32,
    screen_h: f32,
    metrics: &crate::settings::UiMetrics,
) -> Option<PopupMenu>
~~~

函数体开头改为：

~~~rust
let dpi = metrics.dpi;
let show_line_numbers = input.show_line_numbers;
let word_wrap = input.word_wrap;
let show_status_bar = input.show_status_bar;
let theme_mode = input.theme_mode;
let current_mode = input.view_mode;
~~~

后续 item_h/menu_w/anchor/padding 继续只乘 dpi；所有 is_active 分别使用以上行为局部变量。

同一任务迁移 app_dispatch.rs 的直接 builder 调用，避免新签名提交后 app 无法编译：

~~~rust
let metrics = self.ui_metrics();
let sidebar_settings =
    ui::widgets::sidebar::types::SidebarSettingsInput::from(&self.settings);
self.ui_shell.sidebar_persistent.open_menu =
    ui::widgets::sidebar::build_settings_menu(
        btn_rect_opt,
        &sidebar_settings,
        sw,
        sh,
        &metrics,
    );
~~~

- [ ] **Step 4: 运行 sidebar menu 测试**

~~~bash
cargo test -p edit-plus-ui --lib widgets::sidebar::menu -- --nocapture
cargo test -p edit-plus-ui --lib widgets::sidebar::types -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/ui/src/widgets/sidebar/types.rs crates/ui/src/widgets/sidebar/menu.rs crates/app/src/app_dispatch.rs
git commit -m "refactor(ui): separate sidebar behavior input"
~~~

### Task 10: 迁移 SidebarState 与 SidebarWidget

**Files:**
- Modify/Test: crates/ui/src/widgets/sidebar/state.rs
- Modify/Test: crates/ui/src/widgets/sidebar/mod.rs
- Modify: crates/app/src/app_dispatch.rs

- [ ] **Step 1: 写 menu active 状态更新失败测试**

在 state.rs 增加：

~~~rust
#[test]
fn settings_menu_uses_latest_behavior_input() {
    let cfg = SidebarConfig::new_default(2.0);
    let mut state = SidebarState::new(&cfg);
    let metrics = UiMetrics::from_settings(&Settings::new(), 2.0);
    let input = SidebarSettingsInput {
        show_line_numbers: false,
        word_wrap: false,
        show_status_bar: true,
        theme_mode: crate::settings::ThemeMode::Dark,
        view_mode: crate::view_mode::ViewMode::Tabs,
    };

    state.open_settings_menu(800.0, 600.0, &metrics, &input);
    let menu = state.open_menu.as_ref().unwrap();
    assert!(!menu.items[0].is_active);
    assert!(!menu.items[1].is_active);
    assert!(menu.items[2].is_active);
    assert!(menu.items[5].is_active);
    assert!(menu.items[9].is_active);
}
~~~

- [ ] **Step 2: 运行测试并确认 state 仍从 metrics 读行为**

~~~bash
cargo test -p edit-plus-ui --lib widgets::sidebar::state::tests::settings_menu_uses_latest_behavior_input -- --exact
~~~

Expected: FAIL，open_settings_menu 没有 input 参数。

- [ ] **Step 3: 修改 state 与 widget 数据流**

SidebarState::open_settings_menu 改为：

~~~rust
pub fn open_settings_menu(
    &mut self,
    screen_w: f32,
    screen_h: f32,
    metrics: &crate::settings::UiMetrics,
    input: &SidebarSettingsInput,
) {
    let settings_btn_rect =
        self.layout.as_ref().map(|layout| layout.settings_btn_rect);
    self.open_menu = build_settings_menu(
        settings_btn_rect,
        input,
        screen_w,
        screen_h,
        metrics,
    );
}
~~~

SidebarWidget 增加字段：

~~~rust
settings_input: SidebarSettingsInput,
~~~

sidebar/mod.rs 的 re-export 列表补上 SidebarSettingsInput：

~~~rust
pub use types::{
    SidebarAction, SidebarConfig, SidebarHoverButton, SidebarInput,
    SidebarKey, SidebarSettingsInput, Visibility,
};
~~~

SidebarWidget::new 初始化：

~~~rust
settings_input: SidebarSettingsInput::default(),
~~~

保留现有 set_input 签名不变，新增独立行为注入入口：

~~~rust
pub fn set_settings_input(&mut self, settings_input: SidebarSettingsInput) {
    self.settings_input = settings_input;
}
~~~

widget 测试先注入行为，再打开菜单：

~~~rust
let metrics = crate::settings::UiMetrics::from_settings(
    &crate::settings::Settings::new(),
    2.0,
);
let mut widget = SidebarWidget::new(SidebarConfig::new_default(2.0), metrics);
let input = SidebarSettingsInput {
    show_line_numbers: false,
    word_wrap: false,
    show_status_bar: true,
    theme_mode: crate::settings::ThemeMode::Dark,
    view_mode: crate::view_mode::ViewMode::Tabs,
};
widget.set_settings_input(input);
widget.open_settings_menu();
assert_eq!(widget.settings_input, input);
~~~

open_settings_menu 改为：

~~~rust
pub fn open_settings_menu(&mut self) {
    self.state.open_settings_menu(
        self.screen_w,
        self.screen_h,
        &self.metrics,
        &self.settings_input,
    );
}
~~~

布局仍只使用 self.metrics。

app_dispatch.rs 将 Task 9 的临时类型路径：

~~~rust
ui::widgets::sidebar::types::SidebarSettingsInput
~~~

替换为稳定 re-export：

~~~rust
ui::widgets::sidebar::SidebarSettingsInput
~~~

- [ ] **Step 4: 运行 sidebar state/widget 测试**

~~~bash
cargo test -p edit-plus-ui --lib widgets::sidebar::state::tests -- --nocapture
cargo test -p edit-plus-ui --lib widgets::sidebar::widget_tests -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/ui/src/widgets/sidebar/state.rs crates/ui/src/widgets/sidebar/mod.rs crates/app/src/app_dispatch.rs
git commit -m "refactor(ui): inject sidebar settings behavior"
~~~

### Task 11: 由 App 构造并注入 SidebarSettingsInput

**Files:**
- Modify/Test: crates/app/src/ui_shell.rs
- Modify: crates/app/src/app_window.rs

- [ ] **Step 1: 写 Shell 输入透传失败测试**

在 ui_shell.rs 测试模块增加：

~~~rust
#[test]
fn shell_updates_sidebar_with_behavior_input() {
    let sidebar_settings = ui::widgets::sidebar::SidebarSettingsInput {
        show_line_numbers: false,
        word_wrap: false,
        show_status_bar: true,
        theme_mode: ui::settings::ThemeMode::Dark,
        view_mode: ui::view_mode::ViewMode::Tabs,
    };
    let inputs = ShellInputs {
        tabs_visible: false,
        tabs_thickness: 0.0,
        search_visible: false,
        search_thickness: 0.0,
        status_thickness: 0.0,
        sidebar_visible: true,
        sidebar_thickness: 220.0,
        scrollbar_thickness: 0.0,
        toc_visible: false,
        toc_thickness: 0.0,
        metrics: ui::settings::UiMetrics::from_settings(
            &ui::settings::Settings::new(),
            1.0,
        ),
        sidebar_settings,
    };
    assert_eq!(inputs.sidebar_settings, sidebar_settings);
}
~~~

- [ ] **Step 2: 运行测试并确认 ShellInputs 缺字段**

~~~bash
cargo test -p edit-plus-app --lib ui_shell::tests::shell_updates_sidebar_with_behavior_input -- --exact
~~~

Expected: FAIL，sidebar_settings 不存在。

- [ ] **Step 3: 扩展 ShellInputs 并保持单一 metrics DPI**

ShellInputs 改为：

~~~rust
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
    pub sidebar_settings: ui::widgets::sidebar::SidebarSettingsInput,
}
~~~

前置计划已删除 ShellInputs.dpi；本任务保持 LayoutCtx/EventCtx 只读取 inputs.metrics.dpi。UiShell 更新已有 widget 和创建新 widget 时，保持原布局输入调用并紧接着注入行为：

~~~rust
sw.set_input(
    self.sidebar_tabs.clone(),
    self.sidebar_active_index,
    self.sidebar_traffic_light_inset,
    screen.w,
    screen.h,
    &inputs.metrics,
);
sw.set_settings_input(inputs.sidebar_settings);
~~~

app_window.rs 构造输入时：

~~~rust
let metrics = self.ui_metrics();
let sidebar_settings =
    ui::widgets::sidebar::SidebarSettingsInput::from(&self.settings);
~~~

app_dispatch.rs 的直接菜单路径已在 Task 9 原子迁移，本任务不得再次从 metrics 读取行为。

- [ ] **Step 4: 运行 Shell/sidebar/app 编译**

~~~bash
cargo test -p edit-plus-app --lib ui_shell::tests -- --nocapture
cargo test -p edit-plus-ui --lib widgets::sidebar -- --nocapture
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/ui_shell.rs crates/app/src/app_window.rs
git commit -m "refactor(app): inject sidebar behavior through shell"
~~~

### Task 12: 从 UiMetrics 删除行为字段

**Files:**
- Modify/Test: crates/ui/src/settings.rs
- Modify/Test: crates/ui/src/widgets/sidebar/state.rs
- Modify/Test: crates/ui/src/widgets/sidebar/widget_tests.rs

- [ ] **Step 1: 增加 UiMetrics 只含布局字段的编译期构造测试**

将 settings.rs 的 ui_metrics 测试改为显式解构：

~~~rust
let UiMetrics {
    dpi,
    font_size,
    line_height,
    status_bar_height,
    gutter_padding,
    toc_width,
    content_left_margin,
    scrollbar_reserve,
    show_line_numbers,
    show_status_bar,
} = UiMetrics::from_settings(&settings, 2.0);

assert_eq!(dpi, 2.0);
assert_eq!(font_size, settings.font_size * 2.0);
assert_eq!(line_height, settings.line_height * 2.0);
assert_eq!(show_line_numbers, settings.show_line_numbers);
assert_eq!(show_status_bar, settings.show_status_bar);
~~~

该显式解构不包含 word_wrap、theme_mode、view_mode，后续静态扫描保证它们不再位于 UiMetrics 定义。

- [ ] **Step 2: 删除字段并确认残留调用编译失败**

从 UiMetrics 及两个构造器删除：

~~~rust
pub word_wrap: bool,
pub theme_mode: ThemeMode,
~~~

Run:

~~~bash
cargo test -p edit-plus-ui --lib widgets::sidebar --no-run
~~~

Expected: 若 Task 9-11 漏迁移，会在 state/widget tests 中出现 unknown field；修复这些残留后才继续。

- [ ] **Step 3: 将测试 fixtures 全部改为新构造入口**

state.rs 与 widget_tests.rs 顶部各增加：

~~~rust
fn metrics(dpi: f32) -> crate::settings::UiMetrics {
    crate::settings::UiMetrics::from_settings(
        &crate::settings::Settings::new(),
        dpi,
    )
}

fn sidebar_settings() -> SidebarSettingsInput {
    SidebarSettingsInput::from(&crate::settings::Settings::new())
}
~~~

所有 UiMetrics::from(&Settings::new()) 改为 metrics(1.0)，需要 Retina 的测试改为 metrics(2.0)；SidebarWidget::set_input 均补传 sidebar_settings()。

- [ ] **Step 4: 运行全部 UI 测试和静态扫描**

~~~bash
cargo test -p edit-plus-ui --lib
rg -n "word_wrap|theme_mode|view_mode" crates/ui/src/settings.rs
cargo check -p edit-plus-app
~~~

Expected: UI 测试 PASS；扫描只命中 Settings 的行为字段，不命中 UiMetrics 定义或构造器。

- [ ] **Step 5: 提交**

~~~bash
git add crates/ui/src/settings.rs crates/ui/src/widgets/sidebar/state.rs crates/ui/src/widgets/sidebar/widget_tests.rs
git commit -m "refactor(ui): keep behavior out of metrics"
~~~

### Task 13: 迁移 UiShell 与 TabBar 的 UiMetrics 测试 fixtures

**Files:**
- Modify/Test: crates/app/src/ui_shell.rs
- Modify/Test: crates/ui/src/widgets/tab_bar/widget.rs

- [ ] **Step 1: 建立统一测试 helper**

ui_shell.rs 测试模块增加：

~~~rust
fn metrics(dpi: f32) -> ui::settings::UiMetrics {
    ui::settings::UiMetrics::from_settings(
        &ui::settings::Settings::new(),
        dpi,
    )
}
~~~

tab_bar/widget.rs 测试模块增加：

~~~rust
fn metrics(dpi: f32) -> crate::settings::UiMetrics {
    crate::settings::UiMetrics::from_settings(
        &crate::settings::Settings::new(),
        dpi,
    )
}
~~~

- [ ] **Step 2: 机械替换旧构造并运行静态扫描**

将两个文件中的：

~~~rust
UiMetrics::from(&Settings::new())
~~~

替换为：

~~~rust
metrics(1.0)
~~~

Run:

~~~bash
rg -n "UiMetrics::from\(" crates/app/src/ui_shell.rs crates/ui/src/widgets/tab_bar/widget.rs
~~~

Expected: 无输出。

- [ ] **Step 3: 运行对应测试**

~~~bash
cargo test -p edit-plus-app --lib ui_shell::tests -- --nocapture
cargo test -p edit-plus-ui --lib widgets::tab_bar -- --nocapture
~~~

Expected: PASS。

- [ ] **Step 4: 编译检查**

~~~bash
cargo check -p edit-plus-app
~~~

Expected: PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/ui_shell.rs crates/ui/src/widgets/tab_bar/widget.rs
git commit -m "test(ui): construct metrics from explicit dpi"
~~~

### Task 14: 迁移搜索与编辑 dispatch 的物理行高

**Files:**
- Modify/Test: crates/app/src/app_search.rs
- Modify/Test: crates/app/src/dispatch/editor.rs
- Modify/Test: crates/app/src/dispatch/mouse.rs

- [ ] **Step 1: 为三个遗漏消费者增加源码边界失败测试**

每个文件的测试模块增加同结构测试；以下为 app_search.rs，另外两个文件分别把 `include_str!` 路径改为 `"editor.rs"` 和 `"mouse.rs"`：

~~~rust
#[test]
fn production_code_does_not_read_logical_line_height_for_geometry() {
    let source = include_str!("app_search.rs");
    let production = source.split("#[cfg(test)]").next().unwrap_or(source);
    let forbidden = ["self.settings", ".line_height"].concat();
    assert!(!production.contains(&forbidden), "found {forbidden}");
}
~~~

- [ ] **Step 2: 运行测试并确认三条直接读取均被捕获**

~~~bash
cargo test -p edit-plus-app --lib app_search::tests::production_code_does_not_read_logical_line_height_for_geometry -- --exact
cargo test -p edit-plus-app --lib dispatch::editor::tests::production_code_does_not_read_logical_line_height_for_geometry -- --exact
cargo test -p edit-plus-app --lib dispatch::mouse::tests::production_code_does_not_read_logical_line_height_for_geometry -- --exact
~~~

Expected: 三个测试均 FAIL，分别命中搜索跳转、编辑完成后的 viewport、鼠标点击后的 ensure_cursor_visible。

- [ ] **Step 3: 在可变 Workspace 借用前派生一次物理行高**

三个文件的每个 App handler 在取得 `active_doc_mut`/`active_view_mut` 前统一执行：

~~~rust
let line_height = self.ui_metrics().line_height;
~~~

并按映射替换：

~~~text
self.settings.line_height -> line_height
~~~

同一 handler 内只派生一次；不得在持有 document 可变借用后调用 self.ui_metrics()。

- [ ] **Step 4: 运行 dispatch/search 测试和编译**

~~~bash
cargo test -p edit-plus-app --lib app_search::tests -- --nocapture
cargo test -p edit-plus-app --lib dispatch::editor::tests -- --nocapture
cargo test -p edit-plus-app --lib dispatch::mouse::tests -- --nocapture
rg -n "self\.settings\.line_height" \
  crates/app/src/app_search.rs crates/app/src/dispatch/editor.rs crates/app/src/dispatch/mouse.rs
cargo check -p edit-plus-app
~~~

Expected: 测试和编译 PASS；扫描无输出。

- [ ] **Step 5: 提交**

~~~bash
git add crates/app/src/app_search.rs crates/app/src/dispatch/editor.rs crates/app/src/dispatch/mouse.rs
git commit -m "refactor(app): use metrics in editor follow-up paths"
~~~

### Task 15: 清除初始化、窗口和生命周期的物理尺寸直读

**Files:**
- Modify/Test: crates/app/src/app_init.rs
- Modify/Test: crates/app/src/app_window.rs
- Modify/Test: crates/app/src/app_lifecycle.rs

- [ ] **Step 1: 写三个生产区源码边界测试**

在各文件测试模块加入相同 helper，并分别传入自身 `include_str!`：

~~~rust
fn assert_no_direct_physical_settings(source: &str) {
    let production = source.split("#[cfg(test)]").next().unwrap_or(source);
    for field in ["font_size", "line_height", "status_bar_height", "gutter_padding", "toc_width"] {
        let forbidden = format!("self.settings.{field}");
        assert!(!production.contains(&forbidden), "found {forbidden}");
    }
}

#[test]
fn production_geometry_uses_metrics() {
    assert_no_direct_physical_settings(include_str!("app_window.rs"));
}
~~~

app_init.rs 使用 `include_str!("app_init.rs")`，app_lifecycle.rs 使用 `include_str!("app_lifecycle.rs")`。

- [ ] **Step 2: 运行测试并确认 init_display_map/window/focus 路径失败**

~~~bash
cargo test -p edit-plus-app --lib app_init::tests::production_geometry_uses_metrics -- --exact
cargo test -p edit-plus-app --lib app_window::tests::production_geometry_uses_metrics -- --exact
cargo test -p edit-plus-app --lib app_lifecycle::tests::production_geometry_uses_metrics -- --exact
~~~

Expected: FAIL，至少命中 init_display_map 的 font/line height、build_shell_inputs/window viewport 和焦点恢复 ensure_cursor_visible。

- [ ] **Step 3: 按入口快照迁移**

app_init.rs 的 init_display_map 开头：

~~~rust
let metrics = self.ui_metrics();
let font_size = metrics.font_size;
let line_height = metrics.line_height;
let scrollbar_reserve = metrics.scrollbar_reserve;
~~~

后续 hash、viewport width、clamp/derive 全部使用这些局部值。

app_window.rs 的 build_shell_inputs 开头构造一次 metrics，tabs/search/status/scrollbar/TOC thickness 全部从 metrics 读取；resize/IME/viewport helper 同样在各自入口构造一次 metrics。

app_lifecycle.rs 的 focus/cursor follow-up 在可变 document 借用前执行：

~~~rust
let line_height = self.ui_metrics().line_height;
~~~

并用局部 line_height 调用 ensure_cursor_visible。

- [ ] **Step 4: 增加 sidebar 逻辑持久化转换测试**

在 app_window.rs 测试模块增加：

~~~rust
#[test]
fn sidebar_width_is_persisted_in_logical_units() {
    let mut app = App::new(None);
    app.update_scale_factor(2.0);
    app.ui_shell.sidebar_cfg_mut().width = 440.0;
    let metrics = app.ui_metrics();
    assert_eq!(app.sidebar_width_for_persistence(metrics), 220.0);
}
~~~

实现并供 save_window_geometry 使用：

~~~rust
pub(crate) fn sidebar_width_for_persistence(
    &self,
    metrics: ui::settings::UiMetrics,
) -> f32 {
    self.ui_shell.sidebar_width() / metrics.dpi
}
~~~

- [ ] **Step 5: 运行测试、扫描和编译**

~~~bash
cargo test -p edit-plus-app --lib app_init::tests -- --nocapture
cargo test -p edit-plus-app --lib app_window::tests -- --nocapture
cargo test -p edit-plus-app --lib app_lifecycle::tests -- --nocapture
rg -n "self\.settings\.(font_size|line_height|status_bar_height|gutter_padding|toc_width)" \
  crates/app/src/app_init.rs crates/app/src/app_window.rs crates/app/src/app_lifecycle.rs
cargo check -p edit-plus-app
~~~

Expected: 测试和编译 PASS；生产区扫描无输出，测试中逻辑值断言可保留。

- [ ] **Step 6: 提交**

~~~bash
git add crates/app/src/app_init.rs crates/app/src/app_window.rs crates/app/src/app_lifecycle.rs
git commit -m "refactor(app): finish physical window metrics migration"
~~~

### Task 16: 将兼容阶段测试改为跨语义 facade 断言

**Files:**
- Modify/Test: crates/app/src/app_reshape.rs
- Modify/Test: crates/app/src/app_window.rs
- Modify/Test: crates/app/src/app_lifecycle.rs

- [ ] **Step 1: 改写 Retina zoom 测试，先证明兼容 facade 行为**

app_reshape.rs 的 Retina 测试不得再直接调用 Settings::apply_scale 或 logical_font_size：

~~~rust
#[test]
fn zoom_uses_logical_points_at_retina_scale() {
    let mut app = App::new(None);
    app.update_scale_factor(2.0);

    app.apply_zoom(16.0);
    assert_eq!(app.persisted_font_size(), 16.0);
    assert_eq!(app.ui_metrics().font_size, 32.0);

    app.apply_zoom(15.0);
    assert_eq!(app.persisted_font_size(), 15.0);
    assert_eq!(app.ui_metrics().font_size, 30.0);
}

#[test]
fn zoom_out_clamps_logical_size_at_six() {
    let mut app = App::new(None);
    app.update_scale_factor(2.0);
    app.apply_zoom(6.0);
    assert_eq!(app.persisted_font_size(), 6.0);
    assert_eq!(app.ui_metrics().font_size, 12.0);
}
~~~

- [ ] **Step 2: 迁移 window/lifecycle 旧 DPI 测试**

把两个文件测试中的直接 dpi_scale/apply_scale 设置统一改为：

~~~rust
app.update_scale_factor(2.0);
~~~

物理断言统一读取 app.ui_metrics()，逻辑断言统一读取 app.settings 或 persisted_font_size()。

- [ ] **Step 3: 运行测试并扫描生产文件外的旧用法**

~~~bash
cargo test -p edit-plus-app --lib app_reshape::tests -- --nocapture
cargo test -p edit-plus-app --lib app_window::tests -- --nocapture
cargo test -p edit-plus-app --lib app_lifecycle::tests -- --nocapture
rg -n "settings\.apply_scale|settings\.dpi_scale|logical_font_size|logical_line_height" \
  crates/app/src/app_reshape.rs crates/app/src/app_window.rs crates/app/src/app_lifecycle.rs
cargo check -p edit-plus-app
~~~

Expected: 测试和编译 PASS；扫描无输出。此时 Task 17 的语义切换不需要修改这三个文件即可继续通过。

- [ ] **Step 4: 提交**

~~~bash
git add crates/app/src/app_reshape.rs crates/app/src/app_window.rs crates/app/src/app_lifecycle.rs
git commit -m "test(app): express dpi behavior through metrics facade"
~~~

### Task 17: 原子切换 Settings 为逻辑单位

**Files:**
- Modify/Test: crates/ui/src/settings.rs
- Modify/Test: crates/app/src/app.rs
- Modify/Test: crates/app/src/app_init.rs

- [ ] **Step 1: 写逻辑 Settings 在 DPI 往返中不变的失败测试**

在 app.rs 增加内联 logical_settings_tests 测试模块：

~~~rust
#[cfg(test)]
mod logical_settings_tests {
use super::App;

#[test]
fn scale_factor_changes_metrics_but_not_logical_settings() {
    let mut app = App::new(None);
    let before = (
        app.settings.font_size,
        app.settings.line_height,
        app.settings.status_bar_height,
        app.settings.gutter_padding,
        app.settings.toc_width,
    );

    app.update_scale_factor(2.0);
    let retina = app.ui_metrics();
    assert_eq!(
        (
            app.settings.font_size,
            app.settings.line_height,
            app.settings.status_bar_height,
            app.settings.gutter_padding,
            app.settings.toc_width,
        ),
        before
    );
    assert_eq!(retina.font_size, before.0 * 2.0);

    app.update_scale_factor(1.0);
    assert_eq!(app.ui_metrics().font_size, before.0);
}
}
~~~

在 app_init.rs 测试模块增加持久化值直读测试：

~~~rust
#[test]
fn persisted_font_size_is_loaded_as_logical_value() {
    let persisted = crate::settings_io::PersistedSettings {
        font_size: 18.0,
        ..crate::settings_io::PersistedSettings::default()
    };
    let settings = settings_from_persisted(&persisted);
    assert_eq!(settings.font_size, 18.0);
}
~~~

- [ ] **Step 2: 运行测试并确认 Settings 仍被 apply_scale 修改**

~~~bash
cargo test -p edit-plus-app --lib app::logical_settings_tests::scale_factor_changes_metrics_but_not_logical_settings -- --exact
~~~

Expected: FAIL，font_size/line_height 等仍随 update_scale_factor 改变。

- [ ] **Step 3: 一次性切换 App facade 语义**

app.rs：

~~~rust
pub(crate) fn ui_metrics(&self) -> ui::settings::UiMetrics {
    ui::settings::UiMetrics::from_settings(
        &self.settings,
        self.scale_factor as f32,
    )
}

pub(crate) fn update_scale_factor(&mut self, scale_factor: f64) {
    self.scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
}

pub(crate) fn persisted_font_size(&self) -> f32 {
    self.settings.font_size
}

pub(crate) fn set_logical_font_size(&mut self, logical_size: f32) {
    self.settings.set_font_size(logical_size);
}
~~~

settings.rs 的 Settings::new 和 setter 维持逻辑值，不再由任何 App 路径预缩放。app_init.rs 提取并调用：

~~~rust
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
~~~

display map、TextState 和 worker 的物理值仍由 Task 8 的窗口 metrics 初始化。

- [ ] **Step 4: 运行定向测试、App/UI 全量 lib 测试和编译**

~~~bash
cargo test -p edit-plus-app --lib app::logical_settings_tests::scale_factor_changes_metrics_but_not_logical_settings -- --exact
cargo test -p edit-plus-app --lib app_init::tests::persisted_font_size_is_loaded_as_logical_value -- --exact
cargo test -p edit-plus-ui --lib
cargo test -p edit-plus-app --lib
cargo check -p edit-plus-app
~~~

Expected: PASS；任何 2x 或 1/2x 偏差都阻断 Task 17 提交，不允许加入补偿除法。

- [ ] **Step 5: 提交**

~~~bash
git add crates/ui/src/settings.rs crates/app/src/app.rs crates/app/src/app_init.rs
git commit -m "refactor(settings): store logical dimensions only"
~~~

### Task 18: 删除 DPI 兼容 API 和旧构造入口

**Files:**
- Modify/Test: crates/ui/src/settings.rs

- [ ] **Step 1: 先扫描全部旧 API 调用**

~~~bash
rg -n "settings\.dpi_scale|pub dpi_scale:|apply_scale|logical_font_size|logical_line_height|from_physical_settings|UiMetrics::from\(" crates/app/src crates/ui/src
~~~

Expected: 只允许 settings.rs 中的兼容定义；Tasks 14-16 已清除 app 生产代码和测试中的旧用法。

- [ ] **Step 2: 验证删除前所有消费者已迁移**

~~~bash
cargo test -p edit-plus-app --lib
cargo check -p edit-plus-app
~~~

Expected: PASS；删除兼容定义前 app 已不依赖它们。

- [ ] **Step 3: 删除 Settings DPI 状态和兼容构造**

从 Settings 删除 dpi_scale 字段以及：

~~~rust
Settings::apply_scale
Settings::logical_font_size
Settings::logical_line_height
UiMetrics::from_physical_settings
impl From<&Settings> for UiMetrics
~~~

content_left_margin 与 scrollbar_reserve 从 Settings impl 删除；唯一等价值保留在 UiMetrics::from_settings。

- [ ] **Step 4: 运行静态验收和完整测试**

~~~bash
rg -n "settings\.dpi_scale|pub dpi_scale:|apply_scale|logical_font_size|logical_line_height|from_physical_settings" crates/app/src crates/ui/src
rg -n "UiMetrics::from\(" crates/app/src crates/ui/src
rg -n "word_wrap|theme_mode|view_mode" crates/ui/src/settings.rs
rg -n "self\.settings\.(dpi_scale|font_size|line_height|status_bar_height|gutter_padding|toc_width)" \
  crates/app/src/app_init.rs crates/app/src/app_window.rs crates/app/src/app_lifecycle.rs \
  crates/app/src/app_renderer.rs crates/app/src/render_pipeline.rs \
  crates/app/src/app_scroll.rs crates/app/src/app_search.rs crates/app/src/app_dispatch.rs \
  crates/app/src/events.rs crates/app/src/mouse.rs crates/app/src/dispatch/editor.rs \
  crates/app/src/dispatch/mouse.rs crates/ui/src/decorations.rs
cargo test -p edit-plus-ui --lib
cargo test -p edit-plus-app --lib
cargo check -p edit-plus-app
~~~

Expected:

- 第一、第二条扫描无输出。
- 第三条只命中 Settings 行为字段，不命中 UiMetrics。
- 第四条无输出；这些物理消费者不再直接读取 Settings 尺寸。
- 两个测试命令和编译检查全部 PASS。

- [ ] **Step 5: 提交**

~~~bash
git add crates/ui/src/settings.rs
git commit -m "refactor(settings): remove mutable dpi state"
~~~

### Task 19: 增加回归门禁并验证边界情况

**Files:**
- Modify/Test: crates/app/src/app_tests.rs
- Modify/Test: crates/ui/src/settings.rs
- Modify/Test: crates/app/src/app_lifecycle.rs

- [ ] **Step 1: 增加 DPI/Zoom/sidebar 综合回归测试**

在 app_tests.rs 增加：

~~~rust
#[test]
fn dpi_zoom_and_sidebar_width_are_reversible() {
    let mut app = App::new(None);
    app.workspace.push_view_for_test(crate::view::View::Editor(
        crate::document_view::DocumentView::new(vec!["line".into()], 40, 40.0),
    ));
    let logical_font = app.settings.font_size;
    let logical_sidebar =
        app.ui_shell.sidebar_width() / app.ui_metrics().dpi;
    let settings_version = app.settings.version;
    let reshape_generation = app.reshape_generation;

    app.handle_scale_factor_changed(2.0);
    assert_eq!(app.settings.font_size, logical_font);
    assert_eq!(app.settings.version, settings_version);
    assert!(app.reshape_generation > reshape_generation);
    assert_eq!(app.ui_metrics().font_size, logical_font * 2.0);
    assert_eq!(
        app.ui_shell.sidebar_width(),
        logical_sidebar * 2.0
    );

    app.apply_zoom(logical_font + 1.0);
    assert_eq!(app.settings.font_size, logical_font + 1.0);
    assert_eq!(app.ui_metrics().font_size, (logical_font + 1.0) * 2.0);

    app.handle_scale_factor_changed(1.0);
    assert_eq!(app.settings.font_size, logical_font + 1.0);
    assert_eq!(app.ui_metrics().font_size, logical_font + 1.0);
    assert_eq!(
        app.ui_shell.sidebar_width(),
        logical_sidebar
    );
}

#[test]
fn fractional_dpi_round_trip_does_not_accumulate_error() {
    let mut app = App::new(None);
    let logical = (
        app.settings.font_size,
        app.settings.line_height,
        app.ui_shell.sidebar_width(),
    );
    for dpi in [1.25, 2.0, 1.0] {
        app.handle_scale_factor_changed(dpi);
    }
    assert_eq!(app.settings.font_size, logical.0);
    assert_eq!(app.settings.line_height, logical.1);
    assert!((app.ui_shell.sidebar_width() - logical.2).abs() < 0.001);
}
~~~

- [ ] **Step 2: 增加版本与纯派生测试**

在 settings.rs 增加：

~~~rust
#[test]
fn dpi_derivation_does_not_mutate_settings_version() {
    let settings = Settings::new();
    let version = settings.version;
    let _ = UiMetrics::from_settings(&settings, 2.0);
    assert_eq!(settings.version, version);
}
~~~

App 回归测试中的 settings_version 与 reshape_generation 断言覆盖 DPI 对版本和异步 generation 的影响。

- [ ] **Step 3: 运行失败测试**

~~~bash
cargo test -p edit-plus-app --lib app::app_tests::dpi_zoom_and_sidebar_width_are_reversible -- --exact
cargo test -p edit-plus-app --lib app::app_tests::fractional_dpi_round_trip_does_not_accumulate_error -- --exact
cargo test -p edit-plus-ui --lib settings::tests::dpi_derivation_does_not_mutate_settings_version -- --exact
~~~

Expected: 若 lifecycle 的 sidebar 比例、generation 或非整数倍率处理遗漏则 FAIL；纯派生版本测试应 PASS。render/frame cache 与 shell layout 的具体失效已由 Task 8 的 cache 测试覆盖。

- [ ] **Step 4: 只修正暴露出的单位或失效遗漏**

允许的修正形式：

~~~rust
let old = self.ui_metrics();
self.update_scale_factor(scale_factor);
let new = self.ui_metrics();
self.ui_shell.sidebar_cfg_mut().width *= new.dpi / old.dpi;
self.ui_shell.sidebar_clamp_width(new.dpi);
self.reshape_generation = self.reshape_generation.wrapping_add(1);
self.frame_cache.advance_cache.clear();
self.frame_cache.cluster_pool.clear();
~~~

不允许重新向 Settings 添加 dpi 字段，也不允许在消费者中乘/除 scale factor。

- [ ] **Step 5: 最终验证并提交**

~~~bash
cargo test -p edit-plus-ui --lib
cargo test -p edit-plus-app --lib
cargo check -p edit-plus-app
git diff --check
git add crates/app/src/app_tests.rs crates/ui/src/settings.rs crates/app/src/app_lifecycle.rs
git commit -m "test(settings): guard logical and physical units"
~~~

Expected: 全部 PASS，git diff --check 无输出。

### Task 20: 建立全量 Settings/metrics 边界门禁并完成 workspace 验收

**Files:**
- Create/Test: crates/app/src/settings_boundary_tests.rs
- Modify: crates/app/src/lib.rs
- Modify/Test: crates/app/src/settings_io.rs

- [ ] **Step 1: 写覆盖全部物理消费者的源码门禁**

创建 settings_boundary_tests.rs：

~~~rust
fn production_part(source: &str) -> &str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
}

#[test]
fn physical_consumers_do_not_read_logical_dimensions_directly() {
    let consumers = [
        ("app_init.rs", include_str!("app_init.rs")),
        ("app_window.rs", include_str!("app_window.rs")),
        ("app_lifecycle.rs", include_str!("app_lifecycle.rs")),
        ("app_renderer.rs", include_str!("app_renderer.rs")),
        ("app_reshape.rs", include_str!("app_reshape.rs")),
        ("app_scroll.rs", include_str!("app_scroll.rs")),
        ("app_search.rs", include_str!("app_search.rs")),
        ("app_dispatch.rs", include_str!("app_dispatch.rs")),
        ("render_pipeline.rs", include_str!("render_pipeline.rs")),
        ("mouse.rs", include_str!("mouse.rs")),
        ("dispatch/editor.rs", include_str!("dispatch/editor.rs")),
        ("dispatch/mouse.rs", include_str!("dispatch/mouse.rs")),
    ];
    for (path, source) in consumers {
        let production = production_part(source);
        for field in ["font_size", "line_height", "status_bar_height", "gutter_padding", "toc_width"] {
            let forbidden = format!("self.settings.{field}");
            assert!(!production.contains(&forbidden), "{path}: found {forbidden}");
        }
    }
}

#[test]
fn mutable_dpi_compatibility_api_is_gone() {
    let app = include_str!("app.rs");
    let ui_settings = include_str!("../../ui/src/settings.rs");
    for forbidden in [
        "pub dpi_scale:",
        "fn apply_scale(",
        "fn logical_font_size(",
        "fn logical_line_height(",
        "from_physical_settings",
        "impl From<&Settings> for UiMetrics",
    ] {
        assert!(!app.contains(forbidden), "app contains {forbidden}");
        assert!(!ui_settings.contains(forbidden), "ui settings contains {forbidden}");
    }
}
~~~

- [ ] **Step 2: 注册测试模块并确认遗漏会失败**

在 lib.rs 增加：

~~~rust
#[cfg(test)]
mod settings_boundary_tests;
~~~

运行：

~~~bash
cargo test -p edit-plus-app --lib settings_boundary_tests -- --nocapture
~~~

Expected: 若 Tasks 1-19 有任何遗漏则 FAIL；全部迁移完成后 PASS。

- [ ] **Step 3: 增加 sidebar 逻辑宽度持久化 round-trip**

在 settings_io.rs 测试模块增加：

~~~rust
#[test]
fn physical_sidebar_width_roundtrips_as_logical_value() {
    let mut app = crate::App::new(None);
    app.update_scale_factor(2.0);
    app.ui_shell.sidebar_cfg_mut().width = 440.0;
    let persisted = PersistedSettings {
        sidebar_width: app.sidebar_width_for_persistence(app.ui_metrics()),
        ..PersistedSettings::default()
    };

    let encoded = toml::to_string(&persisted).unwrap();
    let decoded: PersistedSettings = toml::from_str(&encoded).unwrap();
    assert_eq!(decoded.sidebar_width, 220.0);
}
~~~

该测试把 Task 15 的物理→逻辑转换与真实 TOML 序列化/反序列化串在一起，不只验证除法 helper。

- [ ] **Step 4: 运行最终静态扫描**

~~~bash
rg -n "settings\.dpi_scale|pub dpi_scale:|apply_scale|logical_font_size|logical_line_height|from_physical_settings" \
  crates/app/src crates/ui/src
rg -n "UiMetrics::from\(" crates/app/src crates/ui/src
rg -n "self\.settings\.(font_size|line_height|status_bar_height|gutter_padding|toc_width)" \
  crates/app/src/app_init.rs crates/app/src/app_window.rs crates/app/src/app_lifecycle.rs \
  crates/app/src/app_renderer.rs crates/app/src/app_reshape.rs crates/app/src/app_scroll.rs \
  crates/app/src/app_search.rs crates/app/src/app_dispatch.rs crates/app/src/render_pipeline.rs \
  crates/app/src/mouse.rs crates/app/src/dispatch/editor.rs crates/app/src/dispatch/mouse.rs
~~~

Expected: 三条扫描均无输出。逻辑 Settings 的合法读写只允许存在于 settings persistence、App facade 和 settings action/zoom mutation 的明确位置。

- [ ] **Step 5: 运行全 workspace 验收**

~~~bash
cargo fmt --all -- --check
cargo test -p edit-plus-app --lib settings_io::tests::physical_sidebar_width_roundtrips_as_logical_value -- --exact
cargo test --workspace
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets
git diff --check
~~~

Expected: fmt、tests、check、clippy 全部 PASS，git diff --check 无输出；不得用新增 allow 隐藏本计划产生的 warning。

- [ ] **Step 6: 提交**

~~~bash
git add crates/app/src/settings_boundary_tests.rs crates/app/src/lib.rs crates/app/src/settings_io.rs
git commit -m "test(settings): enforce logical physical boundary"
~~~

## 设计覆盖矩阵

| 设计要求 | 实施任务 | 验收证据 |
|---|---:|---|
| Settings 永久保存逻辑尺寸 | 17、18、19 | DPI 往返后字段值与 version 不变；旧字段/API 静态扫描为零 |
| App.scale_factor 是唯一 DPI 状态 | 2、8、15、17、18 | 所有窗口事件经 update_scale_factor；Settings 无 dpi_scale |
| UiMetrics 纯派生且无效 DPI 回退 1 | 1、17、19 | 精确 2x、NaN/Infinity/零/负值、重复派生测试 |
| 帧/事件内使用同一物理快照 | 3-8、11、14、15 | renderer、scroll、events、window、search、dispatch 均在入口构造一次 metrics |
| Zoom 使用逻辑步长 | 2、16、17、19 | DPI 2 下 logical target 与物理字号断言 |
| TextState/display map/worker 启动一致 | 8、15 | InitialWindowMetrics 单快照与初始化调用点测试 |
| DPI 变化缓存与 reshape 失效 | 8、19 | render cache、frame cache、display map、shell layout 与 generation 断言 |
| Sidebar 行为与布局输入分离 | 9-12 | SidebarSettingsInput 测试；UiMetrics 行为字段扫描 |
| Sidebar 瞬态宽度可逆、持久化为逻辑值 | 8、15、19 | 1→1.25→2→1 宽度断言与持久化转换测试 |
| 搜索/编辑/mouse follow-up 使用物理行高 | 14 | 三文件源码边界测试与定向测试 |
| 删除全部兼容层 | 13、16-18、20 | UiMetrics::from 与旧 DPI API 静态扫描为零 |
| 全 workspace 与 all-targets 验收 | 20 | workspace tests、all-targets check、clippy |

## 完成定义

- Settings 中不存在 dpi_scale、apply_scale、logical_font_size 或 logical_line_height。
- App.scale_factor 是窗口 DPI 的唯一状态。
- UiMetrics::from_settings 是唯一 UiMetrics 生产构造入口。
- UiMetrics 不含 word_wrap、theme_mode 或 view_mode。
- 所有布局、绘制、命中测试、viewport 和 reshape request 只消费物理 metrics。
- Zoom 与持久化只读写逻辑 Settings。
- DPI 1→2→1 不改变 Settings 数值或版本；物理 metrics 与 sidebar 瞬态宽度可逆。
- app 与 ui 的 lib 测试、全 workspace tests、all-targets check、clippy 及静态扫描全部通过。

## 实施后非阻断平台手工验证

以下项目用于补充真实窗口系统与 Retina 环境验证，不替代 Task 20 的自动化完成条件，也不阻塞 Phase 2 关闭：

- DPI 输入为 NaN、正负 Infinity、0、负数。
- DPI 1.0→1.25→2.0→1.0 连续变化，避免整数倍率特例掩盖累计误差。
- DPI 事件紧邻 resize、sidebar drag、zoom 和异步 reshape 回包。
- 字号位于 6 与 72 的边界时，在 DPI 1/2 下 zoom clamp 完全一致。
- show_line_numbers/status_bar、theme、view mode 变化只改变行为输入，不改变 metrics 尺寸。
- 无窗口测试环境 scale factor 默认 1.0。
- Retina 下 Markdown、editor、gutter、cursor、selection、TOC、scrollbar 与 popup menu 使用同一 DPI 快照。
