# mmap Adaptive Canvas Viewport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 mmap 增加双向居中的自适应画布、覆盖式横纵滚动条、二维触控板平移，以及保持锚点稳定的独立画布缩放。

**Architecture:** `ui` 提供纯数据画布数学模型和可复用滚动条组件，`app` 在每个 `DocItem` 中保存画布会话状态并把输入事件归约为通用动作，mmap 只报告内容包围盒并消费同一份 `CanvasViewportSnapshot`。mmap 保持未缩放内容布局，绘制与查询通过快照做正逆变换；文字按目标缩放字号重新栅格化，不在 GPU 端拉伸已有字形。

**Tech Stack:** Rust 2024、winit 0.30、现有 `ui::core::DrawList` / Widget 系统、textora 插件协议、Cargo 单元测试。

## Global Constraints

- 遵守 `ui → core/render/shaping`、`app → ui` 的依赖方向；`ui` 不得依赖 `DocumentView`、Workspace、Commands 或 Events。
- `app` 与 `ui` 不得访问 MMF 节点树；跨层只传递矩形、点、缩放、滚动范围和动作等纯数据。
- 初始适配最大 `100%`，自动适配优先不低于 `40%`；手动缩放范围固定为 `25%–400%`。
- 基础内容边距参与缩放；最小屏幕边距、滚动条尺寸和命中热区只跟 DPI 变化。
- 画布视角只在标签页存活期间保存，不写入 mmap、workspace 快照或用户设置。
- 状态使用互斥 `enum`；常量必须语义化；不得使用宽泛命名或 `.unwrap()`。
- 每个任务最多修改三个文件；每次提交前运行指定编译与测试。
- 最终运行 `cargo fmt --all -- --check` 和 `./scripts/verify.sh`。

---

## File Map

- Create `crates/ui/src/canvas.rs`: 画布几何、初始适配、正逆变换与滚动钳制。
- Create `crates/ui/src/widgets/canvas_scrollbars.rs`: 横纵覆盖式滚动条组合组件。
- Create `crates/app/src/canvas_viewport.rs`: 每标签页画布会话与动作归约。
- Modify `crates/ui/src/widgets/scrollbar.rs`: 把纵向滚动条泛化为轴向滚动条。
- Modify `crates/ui/src/plugin.rs`: 增加画布 Prepare/Render 协议。
- Modify `crates/app/src/tab.rs`: 保存非持久化画布会话。
- Modify `crates/app/src/ui_shell.rs`: 绘制并优先分发画布覆盖层。
- Modify `crates/app/src/app_renderer.rs`: 执行 Prepare → Resolve → Render。
- Modify `crates/app/src/app_window.rs`: 画布不为旧滚动条预留宽度。
- Modify `crates/app/src/dispatch/mouse.rs`: mini-render 复用画布帧准备路径。
- Modify `crates/app/src/events.rs`, `actions.rs`, `app_dispatch.rs`: 路由画布动作。
- Modify `crates/app/src/app_scroll.rs`, `app_lifecycle.rs`: 二维滚动与捏合缩放。
- Modify `crates/markdown/src/mindmap_view.rs`, `mmf/canvas.rs`, `mmf/layout.rs`: mmap 坐标迁移。

---

### Task 1: 纯数据画布数学模型

**Files:**
- Create: `crates/ui/src/canvas.rs`
- Modify: `crates/ui/src/lib.rs`

**Interfaces:**
- Produces: `CanvasPoint`, `CanvasAxis`, `CanvasViewPosition`, `CanvasViewportConfig`, `CanvasViewportInput`, `CanvasViewportSnapshot`, `resolve_viewport()`。
- Consumes: `ui::core::geom::Rect`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn small_content_is_centered_on_both_axes() {
    let snapshot = resolve_viewport(CanvasViewportInput::initial(
        Rect::new(100.0, 50.0, 800.0, 600.0),
        Rect::new(0.0, 0.0, 200.0, 100.0),
        CanvasViewportConfig::for_dpi(1.0),
    ));
    assert_eq!(snapshot.max_scroll, CanvasPoint::ZERO);
    assert_eq!(snapshot.content_to_screen(CanvasPoint::ZERO), CanvasPoint::new(400.0, 300.0));
}

#[test]
fn initial_fit_stops_at_readable_floor() {
    let snapshot = resolve_viewport(CanvasViewportInput::initial(
        Rect::new(0.0, 0.0, 320.0, 240.0),
        Rect::new(0.0, 0.0, 2_000.0, 1_200.0),
        CanvasViewportConfig::for_dpi(1.0),
    ));
    assert_eq!(snapshot.zoom, 0.40);
    assert!(snapshot.max_scroll.x > 0.0 && snapshot.max_scroll.y > 0.0);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-ui --lib canvas::tests -- --nocapture`

Expected: FAIL，`canvas` 模块或类型未定义。

- [ ] **Step 3: 实现类型与算法**

```rust
pub const MIN_MANUAL_ZOOM: f32 = 0.25;
pub const MIN_INITIAL_FIT_ZOOM: f32 = 0.40;
pub const MAX_CANVAS_ZOOM: f32 = 4.0;
pub const DEFAULT_CANVAS_ZOOM: f32 = 1.0;
pub const BASE_CONTENT_PADDING_LOGICAL: f32 = 64.0;
pub const MIN_SCREEN_PADDING_LOGICAL: f32 = 24.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasPoint { pub x: f32, pub y: f32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasAxis { Horizontal, Vertical }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasViewPosition { pub zoom: f32, pub scroll: CanvasPoint }
```

`resolve_viewport()` 先拒绝非有限矩形。初始比例同时满足内容边距与最小屏幕边距：

```rust
let content_padding_fit = viewport_extent / (content_extent + 2.0 * base_padding);
let screen_padding_fit =
    (viewport_extent - 2.0 * min_screen_padding).max(1.0) / content_extent.max(1.0);
let fit_zoom = content_padding_fit.min(screen_padding_fit).min(DEFAULT_CANVAS_ZOOM);
let zoom = fit_zoom.max(config.min_initial_fit_zoom);
```

每个轴使用 `max(base_padding * zoom, min_screen_padding)`。内容可容纳时居中且滚动归零；溢出时以边距为起始 inset。实现 point/rect 正逆变换、position 钳制和安全回退。

- [ ] **Step 4: 补齐负坐标、三种溢出、正逆变换与 NaN 测试**

Run: `cargo test -p textora-ui --lib canvas::tests -- --nocapture`

Expected: PASS。

- [ ] **Step 5: 编译并提交**

```bash
cargo fmt --all -- --check
cargo check -p textora-ui
git add crates/ui/src/canvas.rs crates/ui/src/lib.rs
git commit -m "feat(ui): add canvas viewport geometry"
```

---

### Task 2: 泛化现有滚动条方向

**Files:**
- Modify: `crates/ui/src/widgets/scrollbar.rs`

**Interfaces:**
- Consumes: `CanvasAxis`。
- Produces: `ScrollbarWidget::vertical()`, `horizontal()`, `compute_axis_layout_px()`；`new()` 保持纵向兼容。

- [ ] **Step 1: 写横向布局与拖动失败测试**

```rust
#[test]
fn horizontal_thumb_uses_width_as_primary_extent() {
    let layout = compute_axis_layout_px(
        Rect::new(0.0, 0.0, 400.0, 14.0), 1.0, CanvasAxis::Horizontal,
        100.0, 400.0, 150.0, false,
    );
    assert_eq!(layout.thumb_rect.w, 100.0);
    assert_eq!(layout.thumb_rect.x, 150.0);
    assert_eq!(layout.max_scroll, 300.0);
}
```

再用 MouseDown → MouseMove → MouseUp 验证横向产生 StartDrag、DragTo、EndDrag。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-ui --lib widgets::scrollbar::tests::horizontal -- --nocapture`

Expected: FAIL，横向 API 不存在。

- [ ] **Step 3: 用主轴/交叉轴消除纵向硬编码**

`ScrollbarWidget` 增加 `axis: CanvasAxis`；状态字段改为 `drag_start_pointer_primary` 与 `drag_start_thumb_primary`。纵向 idle thumb 靠右，横向 idle thumb 靠下；翻页、拖动和比例都沿主轴计算。

- [ ] **Step 4: 运行完整滚动条测试并提交**

```bash
cargo test -p textora-ui --lib widgets::scrollbar::tests -- --nocapture
cargo fmt --all -- --check
cargo check -p textora-ui
git add crates/ui/src/widgets/scrollbar.rs
git commit -m "refactor(ui): support horizontal scrollbars"
```

---

### Task 3: 覆盖式双向滚动条组件

**Files:**
- Create: `crates/ui/src/widgets/canvas_scrollbars.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/core/widget.rs`

**Interfaces:**
- Produces: `CanvasScrollbarsInput`, `CanvasScrollbarsAction`, `CanvasScrollbarsWidget`, `WidgetAction::CanvasScrollbars`。

- [ ] **Step 1: 写按需显示和交汇避让失败测试**

```rust
#[test]
fn two_axes_reserve_bottom_right_intersection() {
    let layout = layout_scrollbars(Rect::new(0.0, 0.0, 800.0, 600.0), true, true, 14.0);
    assert_eq!(layout.horizontal.w, 786.0);
    assert_eq!(layout.vertical.h, 586.0);
}
```

单轴输入还需断言缺失方向既不绘制也不命中。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-ui --lib widgets::canvas_scrollbars::tests -- --nocapture`

Expected: FAIL，组件不存在。

- [ ] **Step 3: 实现组合和带轴动作**

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CanvasScrollbarsInput {
    pub horizontal: Option<ScrollbarInput>,
    pub vertical: Option<ScrollbarInput>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanvasScrollbarsAction {
    pub axis: CanvasAxis,
    pub action: ScrollbarAction,
}
```

组件组合横纵两个 `ScrollbarWidget`，使用局部坐标转发绘制与事件；任一子组件拖动时 `is_capturing()` 返回 true。

- [ ] **Step 4: 测试、编译并提交**

```bash
cargo test -p textora-ui --lib widgets::canvas_scrollbars -- --nocapture
cargo fmt --all -- --check
cargo check -p textora-ui
git add crates/ui/src/widgets/canvas_scrollbars.rs crates/ui/src/widgets/mod.rs crates/ui/src/core/widget.rs
git commit -m "feat(ui): add canvas overlay scrollbars"
```

---

### Task 4: 通用画布插件协议

**Files:**
- Modify: `crates/ui/src/plugin.rs`

**Interfaces:**
- Produces: `CanvasContentMetrics`, `ViewPlugin::prepare_canvas()`, `ViewPlugin::render_canvas()`。

- [ ] **Step 1: 写普通插件默认行为失败测试**

测试普通插件 Prepare 返回 None，render_canvas 默认调用原 render，确保非画布实现无需修改。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-ui --lib plugin::tests::canvas_protocol -- --nocapture`

Expected: FAIL，协议方法不存在。

- [ ] **Step 3: 增加可选协议**

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasContentMetrics {
    pub content_bounds: Rect,
    pub focus_anchor: Option<CanvasPoint>,
}

fn prepare_canvas(
    &mut self,
    _doc: &dyn DocView,
    _theme: &Theme,
    _shaper: &mut Shaper,
    _dpi_scale: f32,
) -> Option<CanvasContentMetrics> { None }

fn render_canvas(
    &mut self,
    doc: &dyn DocView,
    viewport: &CanvasViewportSnapshot,
    theme: &Theme,
    shaper: &mut Shaper,
    dpi_scale: f32,
) -> DrawList {
    self.render(doc, viewport.viewport, theme, shaper, dpi_scale)
}
```

内容指标不得加入节点索引、文档范围或 app 类型。

- [ ] **Step 4: 测试、编译并提交**

```bash
cargo test -p textora-ui --lib plugin::tests -- --nocapture
cargo fmt --all -- --check
cargo check -p textora-ui
git add crates/ui/src/plugin.rs
git commit -m "feat(ui): add generic canvas plugin protocol"
```

---

### Task 5: 每标签页画布会话

**Files:**
- Create: `crates/app/src/canvas_viewport.rs`
- Modify: `crates/app/src/lib.rs`
- Modify: `crates/app/src/tab.rs`

**Interfaces:**
- Produces: `CanvasViewportSession`, `CanvasViewportState`, `CanvasViewportAction`。

- [ ] **Step 1: 写状态隔离和锚点稳定失败测试**

```rust
#[test]
fn zoom_keeps_screen_anchor_stable() {
    let mut session = prepared_session();
    let anchor = CanvasPoint::new(420.0, 310.0);
    let before = session.snapshot().screen_to_content(anchor);
    session.apply(CanvasViewportAction::ZoomBy { factor: 1.25, screen_anchor: anchor });
    let after = session.snapshot().screen_to_content(anchor);
    assert_point_close(before, after);
}
```

另测两个 `DocItem` 状态互不污染；指标变化时优先保持 focus anchor，没有焦点时保持旧视口中心。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-app --lib canvas_viewport::tests -- --nocapture`

Expected: FAIL，模块和字段不存在。

- [ ] **Step 3: 实现互斥状态与动作**

```rust
pub(crate) enum CanvasViewportState {
    AwaitingInitialFit,
    Positioned(CanvasViewPosition),
}

pub(crate) enum CanvasViewportAction {
    PanBy(CanvasPoint),
    ZoomBy { factor: f32, screen_anchor: CanvasPoint },
    SetAxisPosition { axis: CanvasAxis, position: f32 },
    Page { axis: CanvasAxis, direction: f32 },
    ResetView,
}
```

会话保存最新有效指标与快照；缺少快照时忽略交互。`DocItem::new()` 创建独立默认会话，不修改 `PersistedTab`。

- [ ] **Step 4: 测试、编译并提交**

```bash
cargo test -p textora-app --lib canvas_viewport::tests -- --nocapture
cargo fmt --all -- --check
cargo check -p textora-app
git add crates/app/src/canvas_viewport.rs crates/app/src/lib.rs crates/app/src/tab.rs
git commit -m "feat(app): store per-tab canvas viewport state"
```

---

### Task 6: UiShell 覆盖层接入

**Files:**
- Modify: `crates/app/src/ui_shell.rs`

**Interfaces:**
- Produces: `UiShell::set_canvas_scrollbars_input()`。

- [ ] **Step 1: 写覆盖层不压缩 editor rect 的失败测试**

设置双向画布滚动条前后比较 `editor_rect()`；点击覆盖条必须返回 `WidgetAction::CanvasScrollbars`，且不落入 Dock。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-app --lib ui_shell::tests::canvas_scrollbars -- --nocapture`

Expected: FAIL，setter 不存在。

- [ ] **Step 3: 接入长期存在的覆盖组件**

`UiShell` 保存组件和可选输入。事件顺序为 popup overlays → canvas scrollbars → Dock；绘制顺序为 Dock chrome → canvas scrollbars → popup overlays。拖动捕获时指针移出轨道仍继续分发。

- [ ] **Step 4: 测试、编译并提交**

```bash
cargo test -p textora-app --lib ui_shell::tests -- --nocapture
cargo fmt --all -- --check
cargo check -p textora-app
git add crates/app/src/ui_shell.rs
git commit -m "feat(app): host canvas overlay scrollbars"
```

---

### Task 7: Prepare → Resolve → Render 帧流程

**Files:**
- Modify: `crates/app/src/app_renderer.rs`
- Modify: `crates/app/src/app_window.rs`
- Modify: `crates/app/src/dispatch/mouse.rs`

**Interfaces:**
- Produces: `App::prepare_active_canvas_frame()`，供正常渲染和 mini-render 共用。

- [ ] **Step 1: 写调用顺序和旧滚动条宽度失败测试**

画布测试插件记录 Prepare/Render 顺序；`build_shell_inputs()` 对 `is_canvas()` 插件断言 `scrollbar_thickness == 0.0`。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-app --lib -- app_renderer canvas_view_does_not_reserve_legacy_scrollbar`

Expected: FAIL，旧右栏仍占宽且没有 Prepare。

- [ ] **Step 3: 实现共享帧准备**

```rust
let metrics = tab.plugin.prepare_canvas(&tab.doc, theme, shaper, dpi)?;
let snapshot = tab.canvas_viewport.resolve(bounds, metrics, CanvasViewportConfig::for_dpi(dpi));
let scrollbars = tab.canvas_viewport.scrollbars_input();
```

画布随后调用 `render_canvas()` 并向 UiShell 注入 scrollbars；Prepare 返回 None 时清除覆盖层并回退原 render。`dispatch/mouse.rs` 的 mini-render 调用同一路径，不得直接绕过快照。

- [ ] **Step 4: 测试、编译并提交**

```bash
cargo test -p textora-app --lib -- app_renderer app_window mouse
cargo fmt --all -- --check
cargo check -p textora-app
git add crates/app/src/app_renderer.rs crates/app/src/app_window.rs crates/app/src/dispatch/mouse.rs
git commit -m "feat(app): resolve canvas viewport before rendering"
```

---

### Task 8: 滚动条动作路由

**Files:**
- Modify: `crates/app/src/actions.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/app/src/app_dispatch.rs`

**Interfaces:**
- Produces: `AppAction::CanvasScrollbar`, `AppAction::CanvasPinch`。

- [ ] **Step 1: 写带轴 DragTo 翻译失败测试**

横向 `CanvasScrollbarsAction::DragTo(320.0)` 必须翻译为横向 `SetAxisPosition`，不能进入旧 `UpdateScrollTop`。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-app --lib events::tests::horizontal_canvas_thumb_drag -- --exact`

Expected: FAIL，AppAction 变体不存在。

- [ ] **Step 3: 增加穷尽路由**

```rust
CanvasScrollbar { axis: CanvasAxis, action: ScrollbarAction },
CanvasPinch { delta: f64, screen_anchor: CanvasPoint },
```

DragTo → SetAxisPosition；PageUp/PageDown → Page(-1/+1)；StartDrag、EndDrag、HoverChanged 只请求重绘。非画布或无快照时返回 `AppEffect::NONE`。

- [ ] **Step 4: 测试、编译并提交**

```bash
cargo test -p textora-app --lib -- events app_dispatch
cargo fmt --all -- --check
cargo check -p textora-app
git add crates/app/src/actions.rs crates/app/src/events.rs crates/app/src/app_dispatch.rs
git commit -m "feat(app): route canvas scrollbar actions"
```

---

### Task 9: 二维滚动、捏合缩放和重置视图

**Files:**
- Modify: `crates/app/src/app_scroll.rs`
- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/dispatch/commands.rs`

**Interfaces:**
- Consumes: `CanvasViewportAction::PanBy`, `ZoomBy`, `AppAction::CanvasPinch`。

- [ ] **Step 1: 写二维 PixelDelta 和修饰键缩放失败测试**

PixelDelta(36, -72) 对画布产生 x=36、y=72 的平移；Cmd/Ctrl + 同一事件只缩放，以当前鼠标位置为锚点。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-app --lib app_scroll::tests::canvas -- --nocapture`

Expected: FAIL，当前 mmap 只消费纵向分量。

- [ ] **Step 3: 在 chrome 路由之后、preview 路由之前处理画布**

```rust
LineDelta(x, y) => CanvasPoint::new(-x * 40.0, -y * 40.0),
PixelDelta(position) => CanvasPoint::new(position.x as f32, -position.y as f32),
```

Shift 且 x=0 时把 y 转为横向。Cmd/Ctrl 时不平移，把 y 转为缩放因子并以 `mouse.pos` 为锚点。winit `WindowEvent::PinchGesture` 分发 CanvasPinch；忽略 NaN，缩放因子为 `(1.0 + delta as f32).max(0.01)`。

当活动插件是画布时，现有 ZoomReset 命令归约为 `CanvasViewportAction::ResetView`；非画布继续执行全局字号重置。ZoomIn/ZoomOut 仍保持现有全局字号语义，避免同一个快捷键在不同视图中改变两类设置。

- [ ] **Step 4: 测试、编译并提交**

```bash
cargo test -p textora-app --lib -- app_scroll app_lifecycle commands
cargo fmt --all -- --check
cargo check -p textora-app
git add crates/app/src/app_scroll.rs crates/app/src/app_lifecycle.rs crates/app/src/dispatch/commands.rs
git commit -m "feat(app): add canvas pan and pinch zoom"
```

---

### Task 10: mmap 迁移到统一快照

**Files:**
- Modify: `crates/markdown/src/mindmap_view.rs`
- Modify: `crates/markdown/src/mmf/canvas.rs`
- Modify: `crates/markdown/src/mmf/layout.rs`

**Interfaces:**
- Consumes: `CanvasContentMetrics`, `CanvasViewportSnapshot`。
- Produces: mmap Prepare/Render 实现和完整内容包围盒。

- [ ] **Step 1: 写缩放命中、光标和拖拽失败测试**

在 50% 与 200%、双轴非零滚动下，分别断言标题命中、CursorScreenPos、IME 矩形和拖拽 pointer 经过正逆变换后仍指向同一内容节点。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p textora-markdown --lib mindmap_view::tests -- --nocapture`

Expected: FAIL，mmap 仍只减 `scroll_y`。

- [ ] **Step 3: 报告完整内容包围盒**

`LayoutTree::content_bounds(selection_outline_gap)` 从节点矩形、连接线端点和选择外扩统一求 min/max，允许负坐标。Prepare 再把当前拖拽 preview、target、guide 与 insertion line 的范围并入当帧指标，保证反馈移到静态布局外时滚动范围同步扩展。边距不得加入内容包围盒。

- [ ] **Step 4: 迁移 MindmapView 状态和查询**

移除 mmap 自有 `scroll_y` 及其 Scroll/SetScrollY/ScrollY/ContentHeight 分支。保存最近成功渲染的 `Option<CanvasViewportSnapshot>`，仅供查询复用。Prepare 复用 `ensure_layout()` 并报告活动节点中心作为 focus anchor；screen hit-test 使用 `screen_to_content()`，光标、IME 和拖拽反馈使用 `content_rect_to_screen()`。

- [ ] **Step 5: 显式映射每类图元**

`mmf/canvas.rs` 用 snapshot 取代 offset_x/offset_y。卡片、圆角、描边、三角形、连接线点和宽度都乘 zoom；文本位置通过 `content_to_screen()`，字体大小使用 `shaper.font_size() * snapshot.zoom`，确保高倍缩放重新栅格化。可见节点裁剪先把屏幕 viewport 逆变换为内容范围。

- [ ] **Step 6: 运行 mmap 与 app 回归测试**

```bash
cargo test -p textora-markdown --lib -- mindmap mmf::canvas mmf::layout
cargo test -p textora-app --lib -- mmap mouse
cargo fmt --all -- --check
cargo check -p textora-markdown
cargo check -p textora-app
```

Expected: 全部 PASS。

- [ ] **Step 7: 提交**

```bash
git add crates/markdown/src/mindmap_view.rs crates/markdown/src/mmf/canvas.rs crates/markdown/src/mmf/layout.rs
git commit -m "feat(markdown): migrate mmap to canvas viewport"
```

---

### Task 11: 跨层回归和完整验证

**Files:**
- Modify: `crates/app/src/app_tests.rs`
- Modify: `crates/markdown/src/mindmap_view.rs`

**Interfaces:**
- Consumes: 全部已实现接口。
- Produces: 生命周期、布局稳定、非法 MMF 和非画布哨兵测试。

- [ ] **Step 1: 增加端到端测试**

测试必须精确验证：标签切换恢复各自 position；新建同路径 DocItem 回到 AwaitingInitialFit；长标题修改前后选中节点屏幕中心差小于 1px；普通 editor/preview 的旧滚动和全局字号缩放保持不变；非法 MMF 不产生 metrics，修复后恢复有效 metrics。

- [ ] **Step 2: 运行针对性测试**

```bash
cargo test -p textora-app --lib -- mmap_canvas mmap_layout non_canvas_plugin
cargo test -p textora-markdown --lib -- invalid_mmap canvas_viewport
```

Expected: 全部 PASS。

- [ ] **Step 3: 清理迁移遗留**

Run: `rg -n "scroll_y|PluginMessage::Scroll|PluginQuery::ContentHeight|offset_x|offset_y" crates/markdown/src/mindmap_view.rs crates/markdown/src/mmf/canvas.rs`

Expected: mmap 不再拥有滚动状态；canvas renderer 不再用 offset 参数实现视口变换。删除对应死代码和未使用 import。

- [ ] **Step 4: 完整验证**

```bash
cargo fmt --all -- --check
cargo test -p textora-ui --lib
cargo test -p textora-markdown --lib
cargo test -p textora-app --lib
cargo check -p textora-app
./scripts/verify.sh
```

Expected: 所有命令退出码为 0；不得通过忽略测试或增加宽泛 lint allow 绕过失败。

- [ ] **Step 5: 提交回归测试**

```bash
git add crates/app/src/app_tests.rs crates/markdown/src/mindmap_view.rs
git commit -m "test: cover mmap adaptive canvas viewport"
```

---

## Manual Acceptance Checklist

- [ ] 小型 mmap 首次打开后双向居中，四周留白均衡。
- [ ] 大型 mmap 不自动缩到 40% 以下，溢出方向出现覆盖式滚动条。
- [ ] 单轴与双轴溢出时滚动条方向正确且不压缩内容视口。
- [ ] 触控板斜向滑动同时产生横向和纵向位移。
- [ ] 双指捏合及 Cmd/Ctrl + 滚轮保持锚点下内容稳定。
- [ ] 缩放范围严格为 25%–400%；重置视图重新适配。
- [ ] 切换标签恢复视角；关闭再打开重新适配。
- [ ] 非 100% 且双向滚动时，节点点击、选择、IME、光标和拖拽均与画面一致。
- [ ] 编辑长标题导致布局变化时，活动节点屏幕位置稳定。
- [ ] 非法 MMF 显示错误画布且无滚动条，修复后恢复。
- [ ] Retina 与普通 DPI 下边距、滑块和命中一致。
