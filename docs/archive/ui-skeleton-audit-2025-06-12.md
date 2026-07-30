# UI 骨架审计报告

> 日期：2025-06-12  
> 范围：2025-06-11 ~ 2025-06-12 提交（Phase 1-9 UI 骨架重构）  
> 最终状态：`e091f0a` (Phase 9 merge)

---

## 〇、提交全景（6.11-6.12）

```
6.12 凌晨 — Phase 9 收尾
  e091f0a Merge phase9-skeleton-cleanup
  e89e218 Task 3: popup_menu NDC 删 + clip ndc→px
  cb3b577 Task 1-2,4-5: 删 NDC 残骸 + UI 状态移入 UiShell
  1a51cd7 Phase 9 收尾清理

6.11 — Phase 1-8 骨架构建
  58edb8f Phase 8 merge: popup overlay 化
  21d49b2 Phase 7: SidebarWidget + widget dispatch    ← 🔴 关键分水岭
  85c969b Phase 6: tab_bar widget 化 + px 化
  d49972e Phase 4-6: SearchBar + Scrollbar + tab_bar
  77cecb2 Phase 3: StatusBarWidget + paint_backend Text
  b4ee525 Phase 2: UiShell + EditorHost + paint_backend
  6a2dee5 Phase 1: geom/paint/measure/widget/dock 就位

6.11 — Sidebar 双模式布局（Phase 7 之前的旧实现）
  880a291 sidebar phases 5-9 (汉堡/悬停/拖拽/设置/标题栏)  ← ✅ 旧代码按钮可工作
  14bd888 sidebar phases 1-4 + text renderer
  b8be3e4 sidebar + tab bar 双模式布局

6.11 — Fixes
  df2bf71 修复滚动条拖拽状态泄漏
  1b0e77d 消除 reshape 自激振荡
  32eaa55 修复渲染刷新过于频繁
  98f1f58 标题栏事件隔离/滚动条范围/行号栏/IME preedit
```

---

## 一、逐条验证结果

验证方法：追踪每个 Bug 的完整代码路径，从事件输入到渲染输出，而非基于推理。

### 1.1 ✅ 确认：Sidebar 不显示 workspace 文档列表

**数据注入断链**：

| 方法 | 定义位置 | app 层调用 |
|------|---------|-----------|
| `set_tabs_input` | `ui_shell.rs:171` | `app_renderer.rs:355` ✅ |
| `set_status_input` | `ui_shell.rs:113` | `app_renderer.rs:311` ✅ |
| `set_scrollbar_input` | `ui_shell.rs:127` | `app_renderer.rs:328` ✅ |
| `set_search_input` | `ui_shell.rs:118` | `app_renderer.rs:320` ✅ |
| `set_sidebar_input` | `ui_shell.rs:157` | **无调用** ❌ |

`set_sidebar_input` 在整个项目中只有一个定义，零个调用点（`grep -rn` 验证）。`UiShell.sidebar_tabs` 永远是 `Vec::new()`，`SidebarWidget` 每帧重建时传入空列表，`VerticalListWidget` 渲染 0 行。

**代码证据** — `app_renderer.rs:335-355`（`set_tabs_input` 调用处）：
```rust
// Phase 6：注入 tab bar 数据
{
    let tab_infos: Vec<ui::tab_bar::TabInfo> = self.workspace.doc_views.iter()
        .map(|dv| ui::tab_bar::TabInfo { title, file_path, is_dirty, language })
        .collect();
    self.ui_shell.set_tabs_input(tab_infos, ...);
}
// ⚠️ 缺少并列的：
// self.ui_shell.set_sidebar_input(cfg, tab_infos, active_index, traffic_light_inset);
```

---

### 1.2 ✅ 确认：汉堡按钮点击无效

**两重阻断**，状态不同原因不同：

**Hidden 状态**（`sidebar_w = 0`）：

```
MouseDown(16, 16), Sidebar mode, Hidden
  → events.rs:281: sidebar_w = sidebar_state.current_width(&cfg)
    → Visibility::Hidden → current_width() 返回 0.0
  → sidebar_w > 0.0? NO — 跳过 sidebar dispatch 块
  → events.rs:299: title_h = 28 * dpi, py=16 < 28? YES
  → return actions（空 Vec）
  → 事件被 title_bar guard 吞掉，零 action
```

**Visible/Pinned 状态**（`sidebar_w > 0`）：

```
MouseDown(16, 16), Sidebar mode, Pinned, sidebar_w = 220
  → events.rs:281: sidebar_w > 0 && px(16) < sidebar_w(220)? YES
  → ui_shell.dispatch(MouseDown(16, 16))
    → Dock::dispatch 逆序遍历 children
    → SidebarWidget::on_event(MouseDown(16, 16))
      → 检查 resize edge? 16 不在 edge 区域 → NO
      → 委派 list.on_event → 16 不在任何 list item 上 → NO
      → 返回 None
  → translate_sidebar_action 从未被调用
  → actions 仍为空
```

**根因**：`SidebarWidget::on_event` 只实现 resize drag 和 list item 点击，遗漏 header 区域的 hamburger/settings/new_btn。这些按钮的命中逻辑在 `SidebarState::hit_test_px` 中存在并通过测试（38 个测试），但 Widget 层不调用它。

---

### 1.3 ✅ 确认：设置按钮点击无效

**与 Bug 1.2 同源 + 额外丢弃**：

- Widget 路径阻断同 Bug 1.2（`on_event` 不处理 settings_btn_rect）
- 即使走通，`translate_sidebar_action` 中显式丢弃：

```rust
// events.rs:504
S::OpenSettingsMenu => None, // handled by old popup path (Phase 8)
```

注释说 Phase 8 接管——但 Phase 8 的 overlay 路径（`events.rs:253-271`）处理的是 **已打开的 popup menu 上的点击**，不负责 **触发 menu 的打开**。`SidebarState::open_settings_menu()` 定义完整但从未有外部调用触发它。

---

### 1.4 ⚠️ 部分确认：滚动条 hover/拖动无效

**Hover 路径正常**：`handle_cursor_moved` 中有 `ui_shell.dispatch(MouseMove)` → Dock → ScrollbarWidget → `hovered` 状态更新 ✅

**拖动路径断裂**：`handle_mouse_input_left` 中 MouseDown 的处理链是：

```
MouseDown(1194, 400), scrollbar area
  → sidebar area? px(1194) > sidebar_w(220) → skip
  → title bar? py(400) > title_h(28) → skip
  → tab bar? py(400) > tbh(32) → skip
  → MouseUp blocks? state.is_pressed() → skip
  → ⚠️ 直接落入 mouse_hit_test → EditorMouseInput
  → ScrollbarWidget::on_event 从未被调用！
```

`handle_mouse_input_left` 缺少一个通用的 `ui_shell.dispatch(MouseDown)` 调用——sidebar 和 tab_bar 有专用 dispatch 块，但 scrollbar (以及未来的 overlay) 没有。Dock 本身的 dispatch 支持 MouseDown，只是 events 层不调用它。

**附加问题**：Phase 9（`cb3b577`）在 `events.rs:337` 删除了 `AppAction::EndScrollbarDrag`，注释 "handled by widget internally"。实际 widget 的 `on_event(MouseUp)` 确实产出 `EndDrag` action，但 events 层收到后不处理——`state.dragging` 在 widget 内部已置 false，但不触发 `needs_redraw`。影响较小（拖动中 MouseMove 持续触发重绘），但语义不正确。

---

### 1.5 ❓ 无法静态确认：行号被遮盖 / 光标偏移 / 选区高亮

经完整追踪 `shape_visible_lines` → `gutter_bg_vertices` → render pass 的坐标链：

- 所有顶点使用屏幕绝对 NDC 坐标（`screen_w`/`screen_h` 基准）
- render pass 无 `set_viewport` / `set_scissor` 调用，使用全屏 viewport
- 示例计算：`screen_w=1200, left_margin=230, gutter_w=50` → gutter NDC x ∈ [-0.7, -0.617]，完全在 [-1,1] 内

**坐标计算路径在静态分析中未发现错误**。如果运行时确实有问题，可能原因：
- 特定 DPI/窗口尺寸下的边界情况
- `content_left_margin` 的默认值过小导致 gutter 与 sidebar 重叠
- 需要运行时截图验证

---

### 1.6 ❓ 无法静态确认：状态栏无信息

**数据链经追踪完整**：

```
app_renderer.rs:311  set_status_input(StatusBarInput { cursor_line, cursor_col, ... })
  → ui_shell.status_input = Some(...)

app_renderer.rs:371  update_frame()
  → ui_shell.rs:280  StatusBarWidget::new()
  → ui_shell.rs:282  sw.set_input(status_input.clone())
  → ui_shell.rs:285  dock.layout() → sw.set_rect() → build_text() → last_text = "5,10"

app_renderer.rs:580  paint_chrome()
  → StatusBarWidget::paint() → DrawCmd::FillRect + DrawCmd::Text("5,10")
  → paint_backend::drain() → emit_text() → GlyphVertex
```

**此路径在代码层面完整无断点**。如果运行时无显示，需排查：字体是否覆盖 ASCII 数字、paint_backend 运行时 text/gpu 是否为 None、或像素坐标是否超出可视区。

---

### 1.7 ❓ 无法静态确认：IME preedit 不显示

渲染路径完整（`app_renderer.rs:553-570` → `preedit_text_vertices`），事件接收正常。唯一阻塞条件：

```rust
if let Some(cursor_vl) = dv.cursor_render_state.cursor_visual_line {
    // 渲染 preedit
}
```

`cursor_visual_line` 在 `render_pipeline.rs` 的 shape 阶段设置（行 198/329/607）。正常使用时（至少一次 render 后）应为 `Some`。仅在首帧 render 前或空文档边界情况下为 `None`。

---

## 二、根因分析

三个确认 Bug 的根因是同一个：**Phase 7 Widget 化不完整**。

```
旧代码 (880a291)                           Phase 7 (21d49b2)
─────────────────────────────────────      ────────────────────────────
events.rs 直接调用                         改为 widget dispatch
  sidebar_state.hit_test_at(px, py)  →      ui_shell.dispatch(ev)
    ├── TogglePin → save + redraw             └── SidebarWidget::on_event
    ├── OpenSettingsMenu → open_menu()              ├── resize edge ✅
    ├── NewDocument → NewEmptyTab                   ├── list items ✅
    ├── SwitchTab → SwitchTab                       ├── hamburger ❌ (未实现)
    └── SetViewMode → SetViewMode                   ├── settings ❌ (未实现)
                                                     └── new_btn  ❌ (未实现)

统一 hit_test_at 覆盖全区域                on_event 只覆盖部分区域
```

`SidebarWidget` 的 `on_event` 是"选择性实现"——只迁移了 resize 和 list item，header 按钮的命中逻辑被遗漏。`SidebarState::hit_test_px` 虽然存在且测试通过，但只被右键路径调用（`events.rs:406`），不被左键 Widget 路径使用。

另外，`events.rs` 的事件分发架构是"挑着分发"而非"统一分发"——sidebar 和 tab_bar 有各自的 dispatch 块，但缺少通用 Dock dispatch 作为 fallback，导致 scrollbar 等 widget 收不到 MouseDown。

---

## 三、修复方案

### 原则

- 不打补丁——不针对单个按钮增加 if-else
- 让架构做正确的事——Dock 已有完善的 hit-test 分发，只需让 events 层使用它
- 消除双轨——`SidebarState` 和 `SidebarWidget` 的命中测试合并

### 具体步骤

**Step 1 — 统一事件路由**（修复 Bug 1.4 + 预防未来问题）

在 `handle_mouse_input_left` 中，所有区域检查之后、editor hit_test 之前，增加通用 Dock dispatch：

```rust
// 通用 widget dispatch：所有未被上述特定区域处理的点击
if state.is_pressed() {
    let dpi = Settings::get().dpi_scale;
    let theme = app.current_theme.clone();
    let mut ctx = EventCtx { theme: &theme, dpi };
    let ev = Event::MouseDown { px, py, button: WidgetMouseButton::Left };
    if let Some(action) = app.ui_shell.dispatch(&ev, &mut ctx) {
        // 处理 scrollbar / overlay / 未来 widget 的 action
        if let Some(sa) = action.downcast_ref::<ScrollbarAction>() { ... }
    }
}
```

**Step 2 — SidebarWidget::on_event 补全**（修复 Bug 1.2 + 1.3）

在 `on_event(MouseDown)` 中，list item 和 resize edge 都不命中时，回退到 `SidebarState::hit_test_px`：

```rust
// 回退到 state 的 hit test（header 按钮等）
if let Some(action) = self.state.hit_test_px(px, py) {
    return Some(Box::new(action));
}
```

**Step 3 — translate_sidebar_action 修正**（修复 Bug 1.3 的翻译层）

- `OpenSettingsMenu` → 实际触发 settings menu overlay
- `TogglePin` → 区分 sidebar toggle vs tab pin（新增 `ToggleSidebar` action）

**Step 4 — 数据注入补全**（修复 Bug 1.1）

在 `app_renderer.rs` 的 `set_tabs_input` 旁增加：

```rust
self.ui_shell.set_sidebar_input(
    self.ui_shell.sidebar_cfg().clone(),
    tab_infos_for_sidebar,  // 复用上面构建的 tab_infos
    Some(self.workspace.active_index),
    traffic_light_inset,
);
```

**Step 5 — 恢复 EndDrag 重绘**（修复 Bug 1.4 附加问题）

在 `handle_mouse_input_left` 的 MouseUp 块中处理 `ScrollbarAction::EndDrag`：

```rust
if let Some(action) = ... {
    if action.downcast_ref::<ScrollbarAction>() == Some(&ScrollbarAction::EndDrag) {
        app.needs_redraw = true;  // 或等价 action
    }
}
```

### 涉及文件

| 文件 | 改动量 | 说明 |
|------|--------|------|
| `app_renderer.rs` | +5 行 | 添加 `set_sidebar_input` 调用 |
| `events.rs` | +30 行 | 通用 Dock dispatch + EndDrag 处理 + translate 修正 |
| `widgets/sidebar.rs` | +10 行 | `on_event` 增加 header 按钮回退 |

---

## 四、测试补充建议

| # | 测试 | 覆盖的问题 |
|---|------|-----------|
| 1 | SidebarWidget: MouseDown on hamburger_rect → ToggleSidebar | Bug 1.2 |
| 2 | SidebarWidget: MouseDown on settings_rect → OpenSettingsMenu | Bug 1.3 |
| 3 | SidebarWidget: MouseDown on new_btn_rect → NewDocument | Bug 1.2 |
| 4 | events: MouseDown on scrollbar thumb → StartDrag → DragTo → EndDrag | Bug 1.4 |
| 5 | app_renderer: set_sidebar_input → sidebar 列表项数量 = doc_views.len() | Bug 1.1 |
| 6 | translate_sidebar_action: OpenSettingsMenu → non-None | Bug 1.3 |
| 7 | Hidden 状态 hamburger (px=16,py=16) → sidebar visibility toggle | Bug 1.2 |

---

## 五、安全性 & 性能

- **unsafe**：UI 层零 unsafe。GPU 通过 wgpu 安全 API
- **RefCell 嵌套借用**：`sidebar.rs` 中 `Settings::get()` 多次独立调用，在特定嵌套路径下可能 panic（已知历史 `2302e3b`/`039e1b1` 修复过类似问题）
- **内存**：每帧重建全部 Widget（~5-6 次 Box::new），开销可接受
- **顶点数**：chrome 渲染的顶点量极小，不影响帧率


---

## 六、代码结构优化建议

以下建议不针对具体 bug，而是从架构层面提升清晰度和可维护性。

### 6.1 合并 SidebarState 与 SidebarWidget（消除双轨）

**现状**：同一个 sidebar 组件分裂为两个 struct，1657 行代码，职责重叠。

```
sidebar.rs (1113 行)                    widgets/sidebar.rs (544 行)
├── SidebarState                        ├── SidebarWidget
│   ├── layout: Option<SidebarLayout>   │   ├── rect, state, cfg
│   ├── visibility                      │   ├── list: VerticalListWidget
│   ├── open_menu, hovered_index        │   ├── tabs, active_index
│   ├── list_scroll_offset, drag        │   ├── dragging, drag_start_*
│   ├── update_layout()                 │   └── set_input(), width(), ...
│   ├── paint()                         │
│   ├── hit_test_px()     ← 只在右键用  │   impl Widget {
│   ├── on_mouse_move()                 │     set_rect() → 调 state.update_layout()
│   ├── on_key()                        │     paint()    → 调 state.paint() + list.paint()
│   ├── tick()                          │     on_event() → resize + list items
│   └── open_settings_menu()            │     hit()      → rect.contains()
│                                       │   }
└── SidebarConfig, SidebarInput,        │
    SidebarAction, SidebarLayout, ...   └── 不处理 header 按钮 ← Bug 根源
```

两个 struct 都持有 tabs/active_index/screen 数据，各自有独立的命中测试（`hit_test_px` vs `on_event`），导致 Bug 1.2/1.3 的发生——header 按钮的命中在 state 中实现但 widget 不调用。

**优化方案**：将 `SidebarState` 完全合并入 `SidebarWidget`：

```rust
pub struct SidebarWidget {
    rect: Rect,
    cfg: SidebarConfig,
    
    // 当前在 SidebarState 中的字段
    visibility: Visibility,
    layout: Option<SidebarLayout>,
    open_menu: Option<PopupMenu>,
    hovered_index: Option<usize>,
    list_scroll_offset: f32,
    drag: Option<EdgeDragState>,
    hover_enter_at: Option<Instant>,
    hover_leave_at: Option<Instant>,
    
    // 子 widget
    list: VerticalListWidget,
    
    // 外部输入
    tabs: Vec<TabInfo>,
    active_index: Option<usize>,
    traffic_light_inset: (f32, f32),
    screen_w: f32,
    screen_h: f32,
}

impl Widget for SidebarWidget {
    fn on_event(&mut self, ev: &Event, _ctx: &mut EventCtx) -> Option<Box<dyn Any>> {
        // 1) resize edge
        // 2) list items (委托 VerticalListWidget)
        // 3) header 按钮 (hamburger / settings / new_btn) — 统一在 hit_test 中处理
        self.hit_test_px(px, py)  // 现在只有一个命中测试入口
    }
}
```

**收益**：
- 代码量从 ~1657 行降至 ~1200 行
- 命中测试只有一处实现，不可能再遗漏
- `set_input` 调一次更新所有状态，不再有同步问题
- `SidebarState` 作为独立类型消失，消除 "用哪个接口" 的困惑

---

### 6.2 事件路由：从"挑着分发"到"统一分发"

**现状**：`handle_mouse_input_left` 有 6 个独立的 if-block，每个硬编码一种 widget 区域：

```
if settings_menu_open → 处理 settings menu
if sidebar_area      → ui_shell.dispatch() → 只取 SidebarAction
if title_bar         → return（吞掉）
if tab_bar_area      → ui_shell.dispatch() → translate_tab_action()
if !pressed          → Sidebar drag end dispatch
if !pressed          → Scrollbar mouse up dispatch
→ mouse_hit_test    （编辑器直接命中测试，绕过 Dock）
```

问题：
- 新增 widget 需要修改 `events.rs`（违反开闭原则）
- scrollbar 的 MouseDown 永远走不到（没有对应的 if-block）
- overlay 的点击也没有通用路径
- Dock 已经有完善的 hit-test 分发，但 events 层不信任它

**优化方案**：信任 Dock 的 hit-test 分发能力，events 层做统一翻译：

```rust
fn handle_mouse_input_left(app: &mut App, state: ElementState, px: f32, py: f32) -> Vec<AppAction> {
    let mut actions = Vec::new();
    
    // 统一入口：所有鼠标事件先走 Dock 分发
    let ev = build_event(state, px, py);
    if let Some(action) = app.ui_shell.dispatch(&ev, &mut ctx) {
        // 翻译层：widget action → AppAction
        translate_widget_action(action, app, &mut actions);
        return actions;
    }
    
    // Dock 未命中 → 落入编辑器
    let hit = editor_hit_test(app, px, py);
    actions.push(AppAction::EditorMouseInput { state, px, py, hit });
    actions
}

fn translate_widget_action(action: Box<dyn Any>, app: &App, actions: &mut Vec<AppAction>) {
    // 统一的 action 翻译，不按来源区分
    if let Some(sa) = action.downcast_ref::<SidebarAction>() { ... }
    if let Some(ta) = action.downcast_ref::<TabAction>() { ... }
    if let Some(sc) = action.downcast_ref::<ScrollbarAction>() { ... }
}
```

**收益**：
- events.rs 从 ~599 行降至 ~200 行
- 新增 widget 不需要修改 events.rs——只需让 widget 产出 action，在翻译层加一个 match arm
- scrollbar/overlay/未来 widget 自动收到事件
- Dock 成为唯一的事件分发入口

---

### 6.3 Widget 生命周期：从"每帧重建"到"持久化 + update"

**现状**：`UiShell::update_frame()` 每帧清空 `dock.children` 并重新 `Box::new` 所有 widget：

```rust
pub fn update_frame(&mut self, ...) {
    self.dock.children.clear();  // 每帧丢弃所有 widget
    
    let mut tbw = TabBarWidget::new();     // ← Box::new
    let mut sw = SearchBarWidget::new();   // ← Box::new
    let mut sbw = StatusBarWidget::new();  // ← Box::new
    let mut siw = SidebarWidget::new(...); // ← Box::new
    let mut scw = ScrollbarWidget::new();  // ← Box::new
    
    // 各自 set_input，然后 push 进 children
    self.dock.layout(screen_rect, &mut layout_ctx);
}
```

问题：
- 每帧 5-6 次堆分配
- widget 内部动画状态无法跨帧保持（如 hover 过渡、scroll 惯性）
- `set_input` 和 `set_rect` 紧耦合——必须先设数据再设布局

**优化方案**：Widget 只创建一次，每帧 update：

```rust
impl UiShell {
    pub fn new() -> Self {
        Self {
            dock: Dock::new(Box::new(EditorHostWidget::new())),
            sidebar: SidebarWidget::new(...),   // 持久化
            scrollbar: ScrollbarWidget::new(),
            status_bar: StatusBarWidget::new(),
            // ...
        }
    }
    
    pub fn update_frame(&mut self, screen: Screen, theme: &Theme, measure: &mut dyn TextMeasure, inputs: &ShellInputs) {
        // 只更新数据，不重建 widget
        self.sidebar.set_input(tabs, active_index, ...);
        self.scrollbar.set_input(viewport_height, total_rows, scroll_top);
        self.status_bar.set_input(status_input);
        
        // 重建 dock layout（轻量，无分配）
        self.dock.children.clear();
        self.dock.children.push(DockChild { widget: &mut self.sidebar, side: Left, ... });
        // ...
        self.dock.layout(screen_rect, ctx);
    }
}
```

**代价**：Dock 需要支持 `&mut dyn Widget` 而非 `Box<dyn Widget>`（所有权改为借用）。这需要修改 Dock 的 children 类型。

**收益**：
- 消除每帧 5-6 次堆分配
- widget 可以持有动画状态、hover 过渡
- `set_input` 和布局解耦，数据注入可以在任意时机

---

### 6.4 Action 系统：从 `Box<dyn Any>` 到类型安全

**现状**：widget 的 `on_event` 返回 `Option<Box<dyn Any>>`，调用方需要 downcast：

```rust
// widget 侧
fn on_event(&mut self, ev: &Event, _: &mut EventCtx) -> Option<Box<dyn Any>> {
    Some(Box::new(ScrollbarAction::DragTo(new_scroll)))
}

// 调用方
if let Some(action) = app.ui_shell.dispatch(&ev, &mut ctx) {
    if let Some(sa) = action.downcast_ref::<ScrollbarAction>() { ... }
    if let Some(sa) = action.downcast_ref::<SidebarAction>() { ... }
}
```

问题：
- 编译期无法检查翻译层是否覆盖了所有 action
- `translate_sidebar_action` 中 `OpenSettingsMenu → None` 是静默丢弃，编译器不告警
- 性能：每次 downcast 都有虚表查找

**优化方案**：定义统一的 `WidgetAction` enum（或使用 trait object 但带 visit 模式）：

```rust
// 方案 A：统一 enum（适合 action 种类有限的场景）
pub enum WidgetAction {
    Sidebar(SidebarAction),
    TabBar(TabBarAction),
    Scrollbar(ScrollbarAction),
    SearchBar(SearchBarAction),
    None,
}

// widget 侧
fn on_event(&mut self, ev: &Event, _: &mut EventCtx) -> WidgetAction {
    WidgetAction::Scrollbar(ScrollbarAction::DragTo(new_scroll))
}

// 翻译层 —— 编译器保证穷尽匹配
match action {
    WidgetAction::Sidebar(sa) => translate_sidebar_action(sa),
    WidgetAction::TabBar(ta) => translate_tab_action(ta),
    WidgetAction::Scrollbar(sc) => translate_scrollbar_action(sc),
    WidgetAction::SearchBar(sb) => translate_search_action(sb),
    WidgetAction::None => {},
}
```

```rust
// 方案 B：回调/visitor 模式（适合 action 种类多的场景）
pub trait ActionHandler {
    fn on_sidebar(&mut self, action: SidebarAction);
    fn on_tab_bar(&mut self, action: TabBarAction);
    fn on_scrollbar(&mut self, action: ScrollbarAction);
}

// 翻译层只需 impl ActionHandler，编译器保证每个方法都被实现
```

**收益**：
- 编译期穷尽检查——不可能静默丢弃 action
- 无需运行时 downcast
- `translate_*_action` 函数的返回类型从 `Option<AppAction>` 变为 `AppAction`（消除了 None 分支）

---

### 6.5 坐标空间显式化

**现状**：渲染管线混合使用两种坐标空间，没有明确的抽象边界：

```
屏幕绝对坐标 (NDC):
  shape_visible_lines → GlyphVertex（NDC = px / screen * 2 - 1）
  gutter_bg_vertices  → GlyphVertex
  cursor_vertices     → GlyphVertex

Dock 布局坐标 (px within screen):
  Dock::layout        → Rect（屏幕 px）
  widget::set_rect    → Rect（屏幕 px）
  widget::paint       → DrawList（屏幕 px）
  paint_backend::drain → 屏幕 px → NDC 转换
```

两种空间本质上相同（都是屏幕物理像素），但 Dock fill_rect 引入了一个"逻辑子区域"概念——编辑器内容应该相对于 fill_rect 渲染，而不是相对于整个屏幕。当前代码通过 `left_margin = content_left_margin + sidebar_offset` 手动补偿，容易出错。

**优化方案**：在 `Dock::layout` 中为 fill widget 计算一个 `LayoutSpace`，传递给渲染管线：

```rust
pub struct LayoutSpace {
    pub rect: Rect,          // 在父空间中的矩形
    pub offset: (f32, f32),  // 累计偏移（从屏幕原点）
}

// shape_visible_lines 只关心自己在 LayoutSpace 内的相对坐标
// 顶点生成时由调用方统一转换到 NDC
fn shape_visible_lines(space: &LayoutSpace, ...) -> Vec<LocalVertex> { ... }
fn local_to_ndc(verts: &[LocalVertex], space: &LayoutSpace, screen: Screen) -> Vec<GlyphVertex> { ... }
```

**收益**：
- 编辑器内容不再需要知道 sidebar 的宽度
- 新增左侧面板时不需要修改 shape 函数
- 坐标变换集中在一处，不会出现双重偏移

---

### 6.6 Settings 依赖注入

**现状**：`Settings::get()` 是全局 `RefCell` 单例，散落在代码各处：

```rust
// sidebar.rs
let dpi = Settings::get().dpi_scale;  // 第 1 次 borrow
// ...
let dpi = Settings::get().dpi_scale;  // 第 5 次 borrow（同一函数内）
```

问题：
- 嵌套 borrow 风险（已知 `2302e3b`、`039e1b1` 修复过）
- 测试需要 `Box::leak` 创建全局 Settings
- 不能并行测试不同 settings 配置

**优化方案**：通过 `ctx` 参数传递 settings，而非全局访问：

```rust
// Widget trait 的上下文已包含 theme 和 dpi
pub struct LayoutCtx<'a> {
    pub measure: &'a mut dyn TextMeasure,
    pub theme: &'a Theme,
    pub settings: &'a Settings,  // ← 新增
}

// 调用方不再需要 Settings::get()
fn update_layout(&mut self, input: &SidebarInput, cfg: &SidebarConfig, dpi: f32) {
    let header_h = HEADER_H * dpi;  // dpi 由调用方传入
    // ...
}
```

**收益**：
- 消除全局可变状态，测试不需要 hack
- 编译器保证生命周期安全
- 不同 widget 可以用不同 Settings 实例

---

### 6.7 优化优先级

| 优先级 | 优化项 | 改动量 | 收益 |
|--------|--------|--------|------|
| P1 | 事件统一路由（6.2） | ~200 行重构 events.rs | 修复 scrollbar bug，简化事件层 |
| P1 | 合并 SidebarState → SidebarWidget（6.1） | ~400 行删 + ~300 行改 | 消除双轨，修复 header 按钮 |
| P2 | Action 类型安全（6.4） | 新增 WidgetAction enum + 重构翻译层 | 编译期穷尽检查 |
| P3 | Widget 持久化（6.3） | 修改 Dock 支持借用 | 减少分配，支持动画 |
| P4 | 坐标空间显式化（6.5） | 新增 LayoutSpace + 重构渲染 | 解耦布局与渲染 |
| P5 | Settings 依赖注入（6.6） | 全局 Settings::get → 参数传递 | 消除全局状态 |

