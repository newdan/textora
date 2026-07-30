# 极简 UI 骨架设计（edit+ ui crate 重构）

- 创建日期：2026-06-11
- 状态：Draft，待用户确认
- 主导诉求：现在 UI 没有整体设计，每个组件各自做绘制和坐标转换；要在不动文本流水线的前提下，给 ui crate 加一层薄骨架，把"共性"沉下来，让组件可以专注于自己的语义。

## 1. 背景与问题

通读 `crates/ui/src/*.rs`（14 个文件，6015 行）+ `crates/app/src/app_renderer.rs` 后，定位到几个核心问题：

1. **没有"UI 整体"的抽象。** 每个组件各自一套：`tab_bar` 1656 行（`TabBarLayout/State/Ctx/Hit/Action`），`sidebar` 464 行（`SidebarLayout/State/Input/Action`），`scrollbar` 660 行（散在 8 个游离函数里），`popup_menu / status_bar / search_bar / gutter / decorations` 风格不统一——有的是 struct + method，有的是裸 fn。
2. **每个组件各自把 px↔NDC 算一遍。** `sidebar.rs:153` 有自己的 `to_ndc` 闭包；`tab_bar / popup_menu / scrollbar / app_renderer` 全都 inline 写 `x / screen_w * 2.0 - 1.0`，至少出现 30+ 处。
3. **"纯数据 vs 调用 GPU/atlas" 边界不清。** UI crate 管 bg quad，app crate 在 `app_renderer.rs::tab_text_vertices`（230 行）和 `popup_menu_text_vertices` 里手工写文字 vertices——两份几乎一样的逻辑。每加一个组件都要在两边各写一份。
4. **容器/层叠/z-order 全靠 `app_renderer::render()` 的顺序记忆。** "顶/底/左"是硬编码 height/width，没人统一算可用编辑器矩形——sidebar 模式下 gutter 起点不正确就是这个问题的体现。popup "在所有东西之上"靠 `vertices.extend` 顺序。
5. **巨型模块。** `tab_bar.rs` 1656、`viewport.rs` 857、`scrollbar.rs` 660；CLAUDE.md 第 4 条要求"改 3 个以上文件就拆"，这些文件本身就该先拆。

## 2. 目标与非目标

**目标**

- ui crate 自给自足：组件输出语义化 `DrawList` 命令，而不是 GPU 顶点。
- 一处坐标系：组件内部全 `Rect`(px)，NDC 仅在 backend 翻译时出现一次。
- 一处文字渲染：app 端的两段 230 行文字顶点逻辑收成一个 backend 函数。
- 显式容器：Dock 模型，吸边布局；`tab_bar / sidebar` 互斥靠 `visible`；剩余空间是 editor。
- 渐进迁移：骨架先就位，老代码不动跑得起来，组件一个一个搬过去。每阶段 `cargo build && cargo test` 通过。

**非目标**

- 不引入 reactive、observer、style sheet、虚拟 DOM。
- 不做 flex 容器（dock 够用）；widget trait 只留扩展点。
- 不动文本流水线：`viewport / layout / decorations / gutter / render_geom / render_pipeline / advance_cache / shape_visible_lines / reshape_worker` 全部不动。
- 不动 `crates/render`、`crates/shaping`、`crates/core`。
- 不引入新依赖；不引入图标 atlas（先用文字字符代替）。

## 3. 总体架构

```
┌─────────────────────────────────────────────────────────────┐
│  ui crate                                                   │
│                                                             │
│  core 层（基建，新增）                                      │
│   ├─ geom.rs        — Rect(px) / Screen / px↔NDC 单一转换    │
│   ├─ widget.rs      — Widget trait, LayoutCtx/PaintCtx/      │
│   │                    EventCtx, Event 枚举, Action 通过     │
│   │                    Box<dyn Any> 上行                     │
│   ├─ dock.rs        — Dock 容器：Top/Bottom/Left/Right + Fill │
│   ├─ paint.rs       — DrawCmd 枚举 + DrawList                │
│   └─ measure.rs     — TextMeasure trait（widget 测文本宽度）  │
│                                                             │
│  widgets 层（具体组件，重构现有）                           │
│   ├─ tab_bar/       — 拆 1656 → layout/state/widget/hit      │
│   ├─ sidebar/       — 删 to_ndc/fill_quad，改 paint           │
│   ├─ status_bar/                                             │
│   ├─ search_bar/                                             │
│   ├─ scrollbar/     — 散函数收进 widget                       │
│   ├─ popup_menu/    — overlay                                │
│   └─ editor_host    — 黑盒 widget，仅持 rect，不渲染          │
│                                                             │
│  保持不动                                                    │
│   ├─ theme / settings / view_mode                            │
│   ├─ viewport / layout / render_geom                         │
│   └─ gutter / decorations                                    │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│  app crate（新增/简化）                                      │
│   ├─ ui_shell.rs (新)    — 持有 root: Dock + overlays         │
│   ├─ paint_backend.rs (新) — DrawList → Vec<GlyphVertex>      │
│   └─ app_renderer.rs       — 简化：只调 shell.paint() + editor│
└─────────────────────────────────────────────────────────────┘
```

ui crate 不持有任何 `wgpu` 类型；DrawList 是被动数据，app backend 决定怎么转 GPU——这给后续替换渲染后端留了口子。

## 4. 核心抽象

### 4.1 几何与屏幕

```rust
// ui::core::geom
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }

impl Rect {
    pub fn contains(self, px: f32, py: f32) -> bool { ... }
    pub fn shrink(self, top: f32, right: f32, bottom: f32, left: f32) -> Rect { ... }
}

#[derive(Copy, Clone, Debug)]
pub struct Screen { pub w: f32, pub h: f32 }

impl Screen {
    pub fn px_to_ndc(self, x: f32, y: f32) -> [f32; 2] {
        [x / self.w * 2.0 - 1.0, 1.0 - y / self.h * 2.0]
    }
    pub fn rect_to_ndc(self, r: Rect) -> [f32; 4] { ... }
}
```

**约束：** ui crate 内部除 `core::geom` 外，禁止出现 NDC 形态的 `[f32; 4]`；widget 的 layout/hit-test 全用 `Rect`(px)。NDC 仅在 `paint_backend` 翻译 DrawList 时出现一次。

### 4.2 DrawList

```rust
// ui::core::paint
pub enum DrawCmd {
    FillRect { rect: Rect, color: [f32; 4], radius: f32 },
    Text { x: f32, y_baseline: f32, font_size: f32,
           color: [f32; 4], content: String },
    PushClip(Rect),
    PopClip,
}

pub struct DrawList { pub cmds: Vec<DrawCmd> }

impl DrawList {
    pub fn fill(&mut self, rect: Rect, color: [f32; 4]);
    pub fn text(&mut self, x: f32, y_baseline: f32,
                font_size: f32, color: [f32; 4], s: &str);
    pub fn clip<F: FnOnce(&mut DrawList)>(&mut self, rect: Rect, f: F);
}
```

`Text` 命令承诺"把这串字画在这里"——widget 不接触 shaper / atlas / font_id，全部由 `paint_backend` 负责。

### 4.3 Widget trait

```rust
// ui::core::widget
pub struct LayoutCtx<'a> {
    pub measure: &'a mut dyn TextMeasure,
    pub theme:   &'a Theme,
    pub dpi:     f32,
}
pub struct PaintCtx<'a> {
    pub list:  &'a mut DrawList,
    pub theme: &'a Theme,
    pub dpi:   f32,
}
pub struct EventCtx<'a> {
    pub theme: &'a Theme,
    pub dpi:   f32,
}

pub enum Event {
    MouseMove { px: f32, py: f32 },
    MouseDown { px: f32, py: f32, button: MouseButton },
    MouseUp   { px: f32, py: f32, button: MouseButton },
    Wheel     { dx: f32, dy: f32, px: f32, py: f32 },
    KeyDown(KeyCode),
}

pub trait Widget {
    fn set_rect(&mut self, rect: Rect, ctx: &mut LayoutCtx);
    fn paint(&self, ctx: &mut PaintCtx);
    fn hit(&self, px: f32, py: f32) -> bool;
    fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx)
        -> Option<Box<dyn Any>> { None }
}
```

**Action 用 `Box<dyn Any>`**：每个 widget 保留自己强类型的 `TabBarAction / SidebarAction / ScrollbarAction`；app 层 downcast。代价是一次堆分配，每帧 0~3 个事件可接受。**好处**：不用维护一个巨型枚举，组件间解耦；新组件不修改公共类型。

### 4.4 TextMeasure

```rust
// ui::core::measure
pub trait TextMeasure {
    fn measure(&mut self, s: &str, font_size: f32) -> f32;
}
```

由 app 实现并注入 `LayoutCtx`。app 端实现包一层 `&mut Shaper`（新建 `app/src/measure_adapter.rs`）：

```rust
pub struct MeasureFromShaper<'a>(pub &'a mut shaping::Shaper);
impl<'a> TextMeasure for MeasureFromShaper<'a> {
    fn measure(&mut self, s: &str, font_size: f32) -> f32 {
        let old = self.0.font_size();
        self.0.set_font_size(font_size);
        let w = self.0.shape(s).map(|r| r.width).unwrap_or(0.0);
        self.0.set_font_size(old);
        w
    }
}
```

### 4.5 DPI 与 Theme 来源（约束）

- DPI 与 Theme **只能**从 `LayoutCtx / PaintCtx / EventCtx` 读，不在 ui crate 内部用 `Settings::get().dpi_scale` 或全局 theme。
- shell 在每帧入口处 `Settings::get().dpi_scale` 读一次，`&app.current_theme` 借一次，灌进 ctx，向下传（不 clone Theme，因其内含 HashMap）。
- 这保证一帧内 dpi/theme 一致；多显示器混合 DPI 时一个窗口对应一个 shell、一个 dpi。
- 全局 `Settings` 其他字段不动；`dpi_scale` 也不删，只是约束 ui crate 不再读它。

### 4.6 Dock 容器

```rust
// ui::core::dock
pub enum Side { Top, Bottom, Left, Right }

pub struct DockChild {
    pub widget:    Box<dyn Widget>,
    pub side:      Side,
    pub thickness: Box<dyn Fn(&Theme, f32 /*dpi*/) -> f32>,
    pub visible:   bool,
}

pub struct Dock {
    pub children: Vec<DockChild>,
    pub fill:     Box<dyn Widget>,
}

impl Dock {
    pub fn layout(&mut self, screen: Rect, ctx: &mut LayoutCtx);
    pub fn paint(&self, ctx: &mut PaintCtx);
    pub fn dispatch(&mut self, ev: &Event, ctx: &mut EventCtx)
        -> Option<Box<dyn Any>>;
}
```

**为什么 thickness 是回调而不是 f32：** sidebar 隐藏时返回 0；新增红绿灯偏移直接改回调；不做缓存以免和 dpi/theme 变化失同步。每帧调用，开销忽略。

**z-order：** 等价于 `Vec` 顺序，没有显式 layer。

**Editor 在 dock 中的位置：** `EditorHostWidget` 作为 `fill`，`set_rect` 时存下来，`paint` 是空操作，`hit` 返回 true（事件落到它）。app 拿到它的 rect 后用旧 `render_pipeline` 自己画。

### 4.7 Overlay 层（popup 等）

popup_menu 不进 dock。`UiShell` 单独维护 overlay：

```rust
pub struct UiShell {
    pub root:     Dock,
    pub overlays: Vec<Box<dyn Widget>>,
}
```

- 事件分发：先 overlays（后入先派发），再 root。
- 绘制顺序：先 root，再 overlays。
- popup 打开时 click 落到 overlay 外要主动关闭（在 overlay widget 自己的 `on_event` 里返回 `Close` action）。

## 5. 默认 dock 装配

```rust
// app/src/ui_shell.rs::build_root()
Dock {
    children: vec![
        DockChild::top   (TabBarWidget::new(),    tab_bar_h),     // visible=show_tabs && view_mode==Tabs
        DockChild::top   (SearchBarWidget::new(), search_h),      // visible=panel_visible
        DockChild::bottom(StatusBarWidget::new(), status_h),
        DockChild::left  (SidebarWidget::new(),   sidebar_w),     // visible=view_mode==Sidebar
        DockChild::right (ScrollbarWidget::new(), scrollbar_w),
    ],
    fill: Box::new(EditorHostWidget::new()),
}
```

`tab_bar` 与 `sidebar` 互斥靠 `visible` 字段，dock 不知道这俩的关系——没有特殊路径。

## 6. 迁移点清单

### A. app crate

| 位置 | 改动 |
|---|---|
| `app/src/ui_shell.rs` | **新建**。`UiShell { root: Dock, overlays }`，提供 `update_frame() / paint() / dispatch()`。`build_root()` 按 §5 装好 widgets |
| `app/src/paint_backend.rs` | **新建**。`fn drain(list: DrawList, screen: Screen, theme, text, gpu) -> Vec<GlyphVertex>`。把现在 `app_renderer.rs::tab_text_vertices`(230 行) 与 `popup_menu_text_vertices`(80 行) 收成一个函数 |
| `app/src/measure_adapter.rs`（新建） | `MeasureFromShaper<'a>(&'a mut Shaper)` 实现 `ui::core::TextMeasure` |
| `app/src/app_renderer.rs::render()` | **大瘦身**：tab text / popup text / status text / sidebar 那几段都删；改成 `shell.update_frame() → shell.paint(&mut list) → backend.drain(list)`；编辑器文本仍走原 `shape_visible_lines` 路径，append 在顶点列表后面 |
| `app/src/app.rs` | `tab_bar_state / sidebar_state / overflow_menu / context_menu` 字段**逐步**搬到 widget 内部；骨架阶段保留旧字段、双写，最终阶段才删 |
| `app/src/events.rs / mouse.rs / input.rs` | 输入事件先翻译成 `ui::core::Event`，统一 `shell.dispatch`；shell 返回 `Box<dyn Any>` 后 app 按 `downcast::<TabBarAction>() / downcast::<SidebarAction>()` 等做 |

### B. ui crate

| 位置 | 改动 |
|---|---|
| `ui/src/lib.rs` | 新增 `mod core { mod geom; mod paint; mod widget; mod dock; mod measure; }` 及 `pub use` |
| `tab_bar.rs`(1656) | 拆为 `tab_bar/{layout, state, widget, hit}`；`Layout` 矩形改 `Rect`；删 `vertices()`，改 `paint`；`TabBarAction` 保留 |
| `sidebar.rs`(464) | 矩形改 `Rect`；删 `to_ndc / fill_quad`；`vertices/text_positions` 合并为 `paint`；`SidebarAction` 保留 |
| `scrollbar.rs`(660) | `compute_layout / hit_test / handle_*` 散函数收进 `ScrollbarWidget` |
| `popup_menu.rs`(246) | 改成 widget；放进 overlays |
| `status_bar.rs / search_bar.rs` | 按 widget 改写 |
| `gutter.rs / decorations.rs` | **不动**（文字几何工具，不是 widget） |
| `viewport.rs / layout.rs / render_geom.rs / theme.rs / settings.rs / view_mode.rs` | **不动** |

### C. 不改的范围（明确划线）

- `crates/render`、`crates/shaping`、`crates/core`：完全不动。
- 文本几何：换行算法、advance cache、cluster 处理、cursor/selection/search 高亮的几何计算——全留在原位。
- 编辑器主区域渲染管线（`render_pipeline.rs / shape_visible_lines / advance_cache`）：完全不动；它从 shell 拿到一个 `Rect` 后照旧执行。
- 异步重排（`reshape_worker.rs`）、历史/文件 IO、命令系统：不动。

## 7. 阶段切分

每阶段一个 `plans_*.md`，结束都能 `cargo build && cargo test`。

1. **骨架就位**：`core/{geom, paint, widget, dock, measure}` 五个文件 + `MeasureFromShaper` 适配；ui_shell + paint_backend 建空壳。老代码继续跑，不接入。
2. **dock + editor host**：完成 Dock 的 layout/paint/dispatch；EditorHostWidget；shell 真接管 layout，但所有 chrome widget 仍是空 widget。app_renderer 仍走老路径——验证 dock 算的 rect 与旧逻辑一致。
3. **status_bar 试水**（最小、最孤立）：第一个真 widget；删 `app_renderer.rs::status_bar_bg_vertices / status_bar_text_vertices`。
4. **search_bar**：第一个带键盘事件、有显隐切换的 widget。
5. **scrollbar**：第一个带拖拽状态的 widget。
6. **tab_bar 拆分 + widget 化**：1656 → 4 文件；删 `tab_text_vertices`(230 行)。
7. **sidebar widget 化**：删 `to_ndc / fill_quad`。
8. **popup_menu overlay 化**：从 tab_bar / sidebar 抽出，进 `UiShell::overlays`；删 `popup_menu_text_vertices`(80 行)。
9. **清理**：删 `app.rs` 双写字段、删旧 vertices 函数、整理 `ui::lib.rs`。

## 8. 边界情况

1. **`show_tabs == false`（单文档）**：`tab_bar.visible = false`，dock 跳过，editor rect 顶屏幕顶。
2. **sidebar 拖拽 resize**：宽度回调每帧读 `cfg.width`，不缓存。
3. **search_bar 显隐**：`visible = false` 时 thickness 调用返回 0；显隐切换时无需显式重排。
4. **DPI 变化**：所有 thickness 回调吃 dpi 入参；shell 在 `update_frame` 头部读一次新 dpi 重传。
5. **窗口尺寸变化**：dock 重新 layout；editor host 拿到新 rect 后通知 viewport（`shell.update_frame` 里以回调方式驱动）。
6. **overlay 外点击**：popup widget 在自身 `on_event` 返回 `Close` action；shell 收到后从 overlays 移除。
7. **键盘焦点路由**：search_bar 显示时键盘先到它。当前由 app 显式判断 `search_state.panel_visible`；迁移后 shell 维护 `keyboard_focus: Option<WidgetId>`，键盘事件优先派发给焦点 widget。
8. **多显示器混合 DPI**：一个窗口一个 shell；ctx.dpi 取自当前窗口的 `Settings`，frame-scoped 读取保证一致。

## 9. 测试

- `core::dock`：
  - `dock_layout_top_then_bottom_leaves_correct_fill`（顶 32 + 底 24 → fill = `screen.h - 56`）
  - `dock_invisible_child_does_not_consume_space`
  - `dock_dispatch_routes_to_topmost_hit`
- `core::paint`：
  - `clip_pop_pair_invariant`（DrawList 永远配对）
  - `text_command_carries_baseline_not_top`
- `widgets/*`：每个 widget 至少一组 paint 断言（DrawList 命令序列）+ 至少一组事件 → action 断言。
- 迁移阶段回归：
  - `tab_bar_widget_paint_matches_old_vertices`（同输入下，新 widget 的 DrawList 经 backend 翻译后的顶点序列等价于旧 `tab_text_vertices`）。
  - 类似地为 sidebar / popup_menu 写回归。

## 10. 取舍与风险

- **`Box<dyn Any>` 上行 action**：每帧 0~3 个事件，可接受；好处是组件解耦。
- **DrawList 中 `Text` 携带 `String`**：每帧分配。MVP 接受；后期可换 `Cow<'a, str>` 或字符串池。
- **`Box<dyn Fn>` thickness 回调**：每帧调用。开销可忽略（每窗口 5~6 个调用）。
- **测试对全局 `Settings` 的依赖**：迁移阶段会有过渡。spec 第 4.5 节明确约束后，新代码不再引入；旧代码在迁移时清理。
- **migration 期间 ui::tab_bar 双写**：app 里同时存在新 widget 与旧 `TabBarState`。需要 plans_phase6 收口、删除旧字段。

## 11. 不在范围内的事

- 不引入主题系统的层级覆盖（component-level theme override）。
- 不做动画框架（现有 `tick_scroll_animation` 留在原处）。
- 不做无障碍/IME 焦点管理（`keyboard_focus` 仅为最小路由）。
- 不做拖放 reorder 框架（tab 拖拽如已存在，留在 widget 内部）。

## Phase 1 完工记录（追加）

- 完工日期：2026-06-11
- 提交范围：commit `98f1f58` ~ 待提交
- 已建立的骨架：
  - `ui::core::geom`（`Rect / Screen`）
  - `ui::core::paint`（`DrawCmd / DrawList`）
  - `ui::core::measure`（`TextMeasure / NoopMeasure`）
  - `ui::core::widget`（`Widget` trait + ctx + Event）
  - `ui::core::dock`（吸边布局 + dispatch）
  - `app::measure_adapter`（`MeasureFromShaper`）
- 老代码完全未接入；下阶段 Phase 2 起接入 EditorHost + UiShell 骨架。


## Phase 2 完工记录

- 完工日期：2026-06-11
- 提交范围：Phase 2（editor_host / paint_backend / ui_shell / app 接入）
- 新增文件：
  - `crates/app/src/editor_host.rs` — 黑盒 widget，仅记 rect（4 测试）
  - `crates/app/src/paint_backend.rs` — DrawList → GlyphVertex（FillRect+Clip，8 测试）
  - `crates/app/src/ui_shell.rs` — Dock 封装 + ShellInputs + update_frame/paint/dispatch（6 测试）
- 修改文件：
  - `crates/app/src/lib.rs` — +3 pub mod
  - `crates/app/src/app.rs` — +ui_shell 字段、build_shell_inputs() 方法、对齐测试（4 测试）
  - `crates/app/src/app_renderer.rs` — render() 入口处调 update_frame() 双跑
- 接入：UiShell + EditorHostWidget + paint_backend 骨架；Dock 与老路径双跑、editor_rect 与手算一致
- 老代码完全未删；Phase 3 起按 spec §7 顺序依次接入真 widget
- 测试总计：新增 22 测试，全部 525 测试通过，0 失败

---

## Phase 9 完工记录

- 完工日期：2026-06-11
- 提交范围：Phase 9（收尾清理）

### Task 1：删 tab_bar NDC 字段/方法 + 合并 MouseButton
- 删除 TabEntry/TabBarLayout/NavButtonLayout 中所有 NDC 字段
- layout_tabs 全部改为像素空间计算
- 删除 tab_bar_vertices、旧 on_click/hit_test_at/vertices/text_positions 方法
- 删除 tab_bar::MouseButton，统一用 core::widget::MouseButton
- 保留 push_quad/push_rounded_rect/darken（popup_menu 在用）
- 保留 clip_left_ndc/clip_right_ndc（暂需）

### Task 2：删 workspace UI 状态字段（跳过）
- 需独立重构规划

### Task 4：paint_backend 圆角实现
- FillRect radius>0 时走 push_fill 圆角三角扇
- 新增 push_fill + corner_vertex 辅助函数
- 13/13 测试通过

### Task 5：清理无用 AppAction
- 删除 SetScrollbarDragging/UpdateScrollbarState/EndScrollbarDrag/ScrollbarHovered

### 测试结果
- UI 层 295 测试通过
- App 层 13 paint_backend 测试通过
- 全 workspace 编译通过
