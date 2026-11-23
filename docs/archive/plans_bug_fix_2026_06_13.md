# 修改方案：内容区布局 / 滚动条 / 软换行 / 多行选择 / IME 五项 Bug

> 日期：2026-06-13
> 范围：5 个独立 Bug，分阶段交付，互不耦合。
> **本文档只描述根因与方案，不含代码改动。**

---

## 阶段 0 — 总览

| ID  | Bug                              | 严重度 | 影响模块                       | 阶段 |
| --- | -------------------------------- | ------ | ------------------------------ | ---- |
| B1  | 状态栏越过 sidebar / 滚动条；sidebar+titlebar 模式下 scrollbar/titlebar 重叠 | 高 | `app/src/ui_shell.rs`（Dock 顺序）+ 新增 TitleBarSpacer | 1 |
| B2  | 滚动条 hover/drag 视觉无效；滚动条粗细未切换 | 高 | `app/src/events.rs`、`app/src/app.rs`、`ui/src/widgets/scrollbar.rs`、`ui/src/scrollbar.rs` | 2 |
| B3  | 软换行点击后光标偏上一行         | 高     | `app/src/render_pipeline.rs`     | 3    |
| B4  | 多行选择第二行起高亮少一字       | 中     | `ui/src/render_geom.rs`、`ui/src/layout.rs`、`app/src/commands.rs` 等 | 4 |
| B5  | IME 输入未显示 preedit 字母      | 高     | `app/src/app_lifecycle.rs`、`events.rs` | 5    |

阶段切分原则：每个 Bug 独立一个阶段，单独可上线，互不依赖。每个阶段的工作量都在「单文件级别」可以一次性完成。

---

## 阶段 1 — B1：内容区布局边界（含 sidebar+titlebar）

### 现象
- 状态栏（Status Bar）目测延伸到 Sidebar 上方，没有"窝"在内容区里。
- 滚动条位置看起来不对。

### 项目两种 view_mode

文件：`crates/app/src/app.rs::build_shell_inputs` (行 172–225) 与 `app.rs::content_top_offset` (行 146–154)

| view_mode | 顶部 chrome     | 实现方式                                                              |
| --------- | --------------- | --------------------------------------------------------------------- |
| `Tabs`    | TabBar (Top)    | `Dock::Side::Top` widget（在 Dock 内）                               |
| `Sidebar` | TitleBar (Top)  | **不在 Dock**，由 `app_renderer.rs:381` 直接画 vertices              |

`content_top_offset()` 取 `tab_bar_height` 与 `title_bar_height` 的 max（互斥），因此光标/编辑器内容已经按正确的 top 偏移定位。但 **Dock 不知道 TitleBar 的存在**：

```rust
// app_renderer.rs:381 — 直接发顶点，不通过 dock
ui::title_bar::title_bar_vertices(screen_w, screen_h, dpi, theme, sidebar_left)
```

`title_bar_vertices` 用 `t_ndc = 1.0` 顶到屏幕最上沿；左边界用 `sidebar_left = sidebar_editor_left_offset().max(hamburger_right)`。

因此 Sidebar 模式下：
- **TitleBar 占 y∈[0, 28*dpi]、x∈[sidebar_left, screen_w]**——这是个**绝对坐标重叠层**。
- Dock 内 Sidebar 厚度仅是 `sidebar_editor_left_offset().max(0.5)`（可能很细 / 0.5px 占位）。
- Dock 把整个屏幕高度都给 Scrollbar、StatusBar 切——Scrollbar 顶端会**被 TitleBar 重叠**遮挡。
- StatusBar 即便在底部，也会因当前 push 顺序跨到 sidebar 上方（即用户报的 bug）。

### 根因（更新）

两条独立缺陷叠加：

1. **Dock children push 顺序错**：`ui_shell.rs::rebuild_dock_children` (行 372–471) 中 StatusBar(Bottom) 先于 Sidebar/Scrollbar push。Dock 按顺序切，StatusBar 抢先吃整宽底部 → 跨 sidebar/scrollbar。
2. **Sidebar 模式下 Scrollbar 没躲开 TitleBar 顶层**：Scrollbar 顶 y=0，TitleBar 也覆盖 y∈[0, 28*dpi]，两者在 x∈[sidebar_left, screen_w] 区域**视觉重叠**——TitleBar 后画会盖在 Scrollbar 顶部 thumb 上（或反之导致边角错乱）。

#### 当前矩形 / Sidebar 模式（screen 1200×800, dpi=1.0, sidebar=220, status=24, scrollbar=16, title=28）

| 组件      | x       | y   | w    | h   | 备注                                          |
| --------- | ------- | --- | ---- | --- | --------------------------------------------- |
| TitleBar (overlay 顶层) | 220 | 0  | 980  | 28  | 直接 vertices，**不在 Dock**                  |
| StatusBar | 0       | 776 | 1200 | 24  | ❌ 跨整宽（先 push）                          |
| Sidebar   | 0       | 0   | 220  | 776 | 占左侧（Dock 内）                             |
| Scrollbar | 1184    | 0   | 16   | 776 | ❌ 顶部 0–28 与 TitleBar 重叠                 |
| editor    | 220     | 0   | 964  | 776 | content_top_offset=28，文字从 y=28 开始       |

#### 当前矩形 / Tabs 模式（screen 1200×800, tab=32, status=24, scrollbar=16, sidebar=0）

| 组件      | x   | y   | w    | h   | 备注              |
| --------- | --- | --- | ---- | --- | ----------------- |
| TabBar    | 0   | 0   | 1200 | 32  | OK                |
| StatusBar | 0   | 776 | 1200 | 24  | ❌ 跨整宽（仍错） |
| Scrollbar | 1184| 32  | 16   | 744 | OK                |
| editor    | 0   | 32  | 1200 | 744 |                   |

### 修复方案（合并解决两个问题）

#### 1.1 调整 children push 顺序

把 children push 顺序改为：

```
TabBar(Top) → SearchBar(Top) → Sidebar(Left) → Scrollbar(Right) → StatusBar(Bottom)
```

即 **把 StatusBar 块整体后移到 Scrollbar 之后**。Dock 切割时 Sidebar / Scrollbar 先吃掉左右两列，StatusBar 在剩余的「中间一条」拿底部 24 px。

#### 1.2 Sidebar 模式增加一个顶部 chrome 占位

增加新字段 `ShellInputs.title_thickness: f32`：
- Tabs 模式：0.0
- Sidebar 模式：`title_bar_height_for_dpi(dpi)` (= 28 * dpi)

在 `rebuild_dock_children` **最早**位置（TabBar 之前）push 一个**空 widget**，`Side::Top`、`thickness = title_thickness`、`visible = title_thickness > 0`：

```text
TitleBarSpacer(Top) → TabBar(Top) → SearchBar(Top) → Sidebar(Left) → Scrollbar(Right) → StatusBar(Bottom)
```

注意 Sidebar 模式时 TabBar 不可见（thickness=0，不消耗空间），所以两者互斥不冲突。

TitleBarSpacer 不画任何东西（仍由 `app_renderer.rs:381` 的 vertices 渲染——保持现有路径，避免改动渲染层），它**只是为了让 Dock 知道顶部那 28px 是被占用的**，从而：
- Sidebar 高度从 y=28 起算（不被 TitleBar 盖住顶部）。
- Scrollbar 高度从 y=28 起算，避免与 TitleBar 重叠。
- StatusBar 在剩余区域拿底部，自然只在内容区。

可选：把 spacer 实现为 `Box::new(NoopWidget)`（已有 `StubWidget` 之类，否则新建一个 `EmptyWidget`：paint/hit/on_event 全空）。

#### 修复后矩形 / Sidebar 模式

| 组件                | x   | y   | w    | h   | 备注                          |
| ------------------- | --- | --- | ---- | --- | ----------------------------- |
| TitleBar (overlay)  | 220 | 0   | 980  | 28  | 仍由直接 vertices 画          |
| TitleBarSpacer (Top)| 0   | 0   | 1200 | 28  | Dock 内空 widget（不画）      |
| Sidebar (Left)      | 0   | 28  | 220  | 748 | ✅ 起点 y=28                  |
| Scrollbar (Right)   | 1184| 28  | 16   | 748 | ✅ 不再被 TitleBar 重叠       |
| StatusBar (Bottom)  | 220 | 776 | 964  | 24  | ✅ 仅在内容区                 |
| editor (fill)       | 220 | 28  | 964  | 748 | h 由 776→748（少 28）         |

⚠️ editor_rect 高度从 776 变到 748。但实际之前 `content_top_offset` 已经把内容下推 28px——只是 editor.h 的语义本来就**包含了 TitleBar 区域**（被 overlay 盖掉 28px）。修复后 editor_rect 与可视编辑区**完全一致**，`content_top_offset()` 里 `title_h` 那一支可以删掉（让 Dock 统一管理）。

#### 修复后矩形 / Tabs 模式

| 组件      | x   | y   | w    | h   | 备注              |
| --------- | --- | --- | ---- | --- | ----------------- |
| TabBar    | 0   | 0   | 1200 | 32  | 不变              |
| Scrollbar | 1184| 32  | 16   | 744 | 不变              |
| StatusBar | 0   | 776 | 1200 | 24  | ✅ Sidebar=0 时仍然跨整宽（合理） |
| editor    | 0   | 32  | 1200 | 744 | 不变              |

### 改动点

| 文件                                            | 改动                                                                 |
| ----------------------------------------------- | -------------------------------------------------------------------- |
| `crates/app/src/ui_shell.rs::ShellInputs`       | 新增字段 `title_thickness: f32`                                      |
| `crates/app/src/ui_shell.rs::rebuild_dock_children` | 1) 起首 push TitleBarSpacer(Top); 2) 把 StatusBar 块移到末尾  |
| `crates/app/src/app.rs::build_shell_inputs`     | 计算 `title_thickness`：Sidebar 模式 = `title_bar_height_for_dpi(dpi)`、Tabs 模式 = 0 |
| `crates/app/src/app.rs::content_top_offset`     | （清理）删 `title_h` 分支，统一用 `tab_bar_height`——因为 TitleBar 已经"占位"在 Dock，editor_rect.y 已等于真正起点 |
| 新增 widget：`TitleBarSpacer`                   | 任意 `Box<dyn Widget>` 空实现：paint/hit/on_event 全 noop            |

### 影响 / 测试

- 现有 `sidebar_mode_consumes_left_width` 测试 (`ui_shell.rs:663`) 不传 title 字段——给 `title_thickness=0` 即可保留语义。
- 现有 `tabs_mode_with_scrollbar_status`、`search_bar_below_tabs` 同上。
- **新增**：
  - `sidebar_mode_with_title_pushes_scrollbar_below`：title=28、sidebar=220、scrollbar=16，断言 Scrollbar.y == 28。
  - `status_bar_does_not_cross_sidebar`：sidebar=220、status=24，断言 StatusBar.x ≥ 220。
  - `editor_rect_excludes_title_bar`：sidebar 模式下 editor.y == title_thickness。

### 边界情况

- frame 0 时 sidebar 被守卫跳过，但 title spacer 与 status_bar 与 sidebar 无依赖——单独正常（不再受 sidebar 是否在 dock 影响 status_bar 位置，因为 title spacer 已经把屏幕高度切了 28）。
- TabBar 与 TitleBarSpacer 互斥：thickness 都 > 0 时会一起占顶部——但 `view_mode` 互斥保证不会同时 > 0。
- 拖拽 sidebar 改宽度时 sidebar.h、scrollbar.h、status_bar.x 自动跟随。
- 如果未来要给 TitleBar 增加交互（按钮、菜单），可把 spacer 升级为真正的 `TitleBarWidget` 并把 vertices 渲染搬到 widget paint 里——本次不动。

---

## 阶段 2 — B2：滚动条 hover / drag 视觉无效 + 粗细切换

### 现象
- 鼠标移到滚动条上没有 hover 反馈（轨道不亮）。
- 拖动 thumb 松手后视觉不刷新。
- **滚动条按设计应**：默认是细的（如 4px thumb，无 track），hover 后变粗（如 12–16px thumb + track 高亮）；现在是默认就粗的。

### 根因

#### 2.A 状态变化未触发重绘
文件：
- `crates/app/src/events.rs::translate_scrollbar_action`（行 280–310）
- `crates/app/src/app.rs` 各 AppAction handler（行 683–839 区段）

链路结构是完整的：Dock dispatch → ScrollbarWidget.on_event → 返回 ScrollbarAction → translate → AppAction → app.dispatch。`is_capturing()` 在 dragging 时返回 true，鼠标移出 rect 仍能继续接 MouseMove/MouseUp（`dock.rs:159–169` 已正确）。

**真正缺的是：状态变化未触发重绘**。`translate_scrollbar_action` 对四类动作的处理：

| ScrollbarAction       | 当前 AppAction               | 是否触发 redraw                |
| --------------------- | ---------------------------- | ------------------------------ |
| `HoverChanged(true)`  | `SetCursor(Default)`         | ❌ SetCursor handler 不 redraw |
| `HoverChanged(false)` | （什么都没做）               | ❌                             |
| `StartDrag`           | （什么都没做）               | ❌                             |
| `EndDrag`             | （什么都没做）               | ❌                             |
| `PageUp/PageDown`     | `ScrollViewportBy`           | ⚠️ handler 设了 `needs_redraw=true` 但**没调** `w.request_redraw()` |
| `DragTo`              | `UpdateScrollTop`            | ✅ 完整                        |

后果：Hover 进/出无反馈、EndDrag 后视觉残留、PageUp/PageDown 延迟。

#### 2.B 粗细维度未实现

文件：
- `crates/ui/src/widgets/scrollbar.rs::paint`（行 103–127）
- `crates/ui/src/scrollbar.rs::compute_layout_px`（行 18–44）
- `crates/ui/src/settings.rs::scrollbar_reserve`（行 135–137）= `16.0 * dpi`

当前实现只调透明度：
```rust
let track_alpha = if active { 1.0 } else { 0.0 };
let thumb_alpha = if active { 1.0 } else { 0.4 };
```

`thumb_rect.w = bar_rect.w`（即 `scrollbar_reserve = 16*dpi`）——thumb 永远是 16px 宽。区别只是颜色淡。设计需求是：**几何宽度**也跟随 hover 切换。

VSCode 的常见做法：
- idle: thumb width = 4 * dpi（细条），track 不可见。
- hover/drag: thumb width = 14 * dpi（粗条），track 可见。
- 总 reserve（鼠标拾取宽度）始终 14 * dpi 不变，确保鼠标能稳定命中——细条只是视觉表现，hit-test 仍用 reserve 全宽。

### 修复方案

#### 2.1 `translate_scrollbar_action`：补 redraw（同前）

为四个状态变化分支额外 push `AppAction::RequestRedraw`：
- `HoverChanged(_)`（true 和 false 都要）
- `StartDrag`
- `EndDrag`

如果 `AppAction::RequestRedraw` 暂未定义，新增枚举项（推荐）；处理器只调 `window.request_redraw()`。

#### 2.2 `ScrollViewportBy` handler 调用 `request_redraw()`

文件：`crates/app/src/app.rs:829–839`，对照 `UpdateScrollTop` handler 补一行 `w.request_redraw()`。

#### 2.3 滚动条粗细切换（几何）

新增「设计常量」（建议放 `crates/ui/src/scrollbar.rs` 顶端或 `settings.rs`）：

```text
SCROLLBAR_RESERVE_PX = 14.0    // hit-test 总宽度（不变）
SCROLLBAR_THUMB_W_IDLE = 4.0   // idle thumb 几何宽
SCROLLBAR_THUMB_W_ACTIVE = 14.0 // hover/drag thumb 几何宽
```

改动两处：

**`compute_layout_px`** 增加参数 `active: bool`（或 `thumb_w_px: f32`）：
- 计算 `thumb_w = if active { SCROLLBAR_THUMB_W_ACTIVE * dpi } else { SCROLLBAR_THUMB_W_IDLE * dpi }`。
- thumb 在 bar 内**右对齐**：`thumb_rect = Rect::new(bar_rect.w - thumb_w, thumb_y, thumb_w, thumb_h)`。
- bar_rect.w 仍是 reserve 全宽——hit-test 不变。

**`ScrollbarWidget::set_rect`**（`widgets/scrollbar.rs:91`）调 `compute_layout_px` 时传 `self.state.hovered || self.state.dragging`。

**`ScrollbarWidget::paint`**：
- 删 alpha 切换的 trick；改为：active 时画 track（背景半透明）+ 14px 粗 thumb；idle 时只画 4px 细 thumb（无 track）。
- thumb 颜色：idle 用 `theme.scrollbar_thumb` 但 alpha 略降（如 0.6），active 用 alpha 1.0。

**`Settings::scrollbar_reserve`**：保持 `14.0 * dpi`（如果当前是 16，调到 14 与 active 宽对齐；或保持 16 把 ACTIVE_W 也调成 16——任选其一，关键是两者相等保证不留缝）。

#### 2.4 切换时机：state 变化 → 重新 layout

`set_rect` 仅在 layout 阶段调用一次。hover 状态变化时仅 paint 重画，**`thumb_rect` 不会重算**。两种解法：

- 方案 A（推荐）：把 `compute_layout_px` 移进 `paint`（每帧重算），不再缓存——计算量极小（一次乘除），且消除"layout 缓存与 state 不同步"的隐患。`is_dragging()`、`hit_test on thumb` 仍可通过现算的 layout 工作。
- 方案 B：每次 hover/drag 状态变化时手动调 `set_rect`——侵入性更强，弃用。

采纳方案 A：删掉 `ScrollbarWidget.layout` 字段，改为每次 paint / on_event 时按需 `compute_layout_px(rect, dpi, ..., active)`。

### 影响 / 测试

- 现有 scrollbar widget 单测覆盖状态机本身，不受影响。
- 已有 `paint_idle_emits_thumb_only_no_track` (`scrollbar.rs:283`) 与 `paint_hover_emits_track_and_thumb` (`scrollbar.rs:298`) — 修复后仍通过（idle 1 个 fill, active 2 个 fill）。
- 已有 `dpi_affects_thumb_size` (`scrollbar.rs:485`) 测的是 thumb_h，不受 thumb_w 改动影响。
- **新增**：
  - `idle_thumb_is_thin`：active=false 时 `thumb_rect.w == 4 * dpi`。
  - `hover_thumb_is_thick`：active=true 时 `thumb_rect.w == 14 * dpi`。
  - `thumb_right_aligned`：thumb_rect.x + thumb_rect.w == bar_rect.w。
  - `hover_emits_redraw_request`、`end_drag_emits_redraw_request`。

### 边界情况

- **移动版 / 触屏**：默认细条点击命中范围窄。本设计 hit-test 仍用 reserve 全宽（14px 粗），鼠标命中不缩水——沿用 VSCode 思路。
- **屏幕特别窄（如 < 50 字符）**：reserve 14px 仍合理，无特殊处理。
- **自定义主题**：thumb 颜色已经走 `theme.scrollbar_thumb`，主题适配不破坏。
- **拖拽中 hover 离开**：dragging 优先级高于 hovered（active = hovered || dragging），thumb 保持粗——符合预期。
- 若 `AppAction::RequestRedraw` 暂未抽象，可先在 events.rs 直接对 `app` 引用调用 `app.window.request_redraw()`——但破坏 events.rs 的纯翻译职责。**推荐**：新增 `AppAction::RequestRedraw`。

---

## 阶段 3 — B3：软换行后光标位置偏上一行

### 现象
启用软换行（word wrap）后，鼠标点击某显示行（display visual line, "vl"），光标实际落到上一 vl。

### 根因
文件：`crates/app/src/render_pipeline.rs`

存在**两条**确定 cursor 所在 vl 的代码路径：

#### 路径 A — render_cache 命中路径（行 308–322）

```rust
if cursor_col >= byte_start && cursor_col <= byte_end {
    cursor_vl_in_doc = vli; break;
}
```

非最后行时使用 `<= byte_end`（**边界归左**）。

#### 路径 B — 非缓存路径（行 575–595）

调用 `cursor_motion::find_visual_line_index`，对非最后行使用 `offset < end`（**边界归右**），并在 End 键场景做"归左"修正。

#### 冲突点

软换行下相邻 vl 的字节范围是 *相接* 的：`vl[k].byte_end == vl[k+1].byte_start`。`mouse::hit_test` 在用户点击 wrap 行最左端时会返回 `clusters[vl_start].byte_range.start`——这正好是上一 vl 的 `byte_end`。

- 路径 B（首帧未缓存）：用 `<` → 命中下一 vl，光标位置正确。
- 路径 A（命中缓存，常态）：用 `<=` → 命中上一 vl，**光标偏上一行**。

提交 `914301c0`（"hit_test 逆向插值"）只修了像素插值，没统一这两条路径的归属规则。

### 修复方案

**统一归属规则为"非最后行边界归右"**：

#### 方案 A（推荐）：在 cache 路径调用 `find_visual_line_index`

文件：`crates/app/src/render_pipeline.rs:308–322`

把 cache 命中分支的循环替换成：

```text
let bounds: &[(usize,usize)] = ... // 来源同 cache
cursor_vl_in_doc = cursor_motion::find_visual_line_index(bounds, cursor_col);
```

并参考路径 B 的 End 修正逻辑。

#### 方案 B：抽出 helper

新增 `fn locate_cursor_vl(bounds: &[(usize,usize)], cursor_col: usize, end_affinity: bool) -> usize`，两处调用，杜绝再分叉。

推荐方案 A，最小改动；方案 B 等下次重构时一起做。

### 影响 / 测试

- 既有的非 cache 路径单测继续覆盖路径 B。
- **必须新增**单测：构造一个软换行场景，模拟 cache 命中，断言 cursor 落在用户点击的 vl 而非上一行。
- 同时验证 End 键归左仍然工作（不要把 End 修正一起回归掉）。

### 边界情况

- 硬换行（行尾 `\n`）：每个 doc line 独立 shape，`vl_byte_start = 0`，不存在边界相邻问题，不受影响。
- vl 末位置（buffer 末尾）：不属于"非最后行"，规则保持归左，正确。

---

## 阶段 4 — B4：多行选择第二行起高亮少一字

### 现象
跨多个软换行 vl 选择时，第 1 个 vl 高亮范围正确；从第 2 个 vl 起，**左侧少一个字符宽度**。

### 根因
文件：
- `crates/ui/src/render_geom.rs::byte_to_x`（行 21–42）
- `crates/ui/src/layout.rs::build_advance_cache_entries`（行 6–38）

`AdvanceCacheEntry.clusters` 中存的 `(cluster_end_byte, pixel_x)` 来自 shaper 的 **整行级 byte offset**（`c.byte_range.end`，相对于 doc line 起点而非 vl 起点）。`byte_to_x` 循环时 `prev_end` 初始化为 `0`：

```rust
let mut prev_end: usize = 0;          // ❌ 软换行下第 2 条 vl 起，应等于 vl_byte_start
for &(c_end, c_x) in clusters {
    if c_end > byte_offset { ... }
    let cluster_bytes = c_end.saturating_sub(prev_end);
    let fraction = (byte_offset.saturating_sub(prev_end)) as f32 / cluster_bytes as f32;
    return prev_x + (c_x - prev_x) * fraction;
}
```

对第 2 条 vl 起，`vl_byte_start > 0`，第一个 cluster 的 `c_end = vl_byte_start + N`。当查询 `byte_to_x(vl_byte_start, …, is_end=false)`：

- `cluster_bytes ≈ vl_byte_start + N`（应为 `N`）
- `fraction = vl_byte_start / (vl_byte_start + N)` ≈ 1.0（应为 0）
- 返回值 ≈ 第一个 cluster 的右边缘（应返回 left_margin）

→ 第 2 条 vl 起左边界错向右偏一个字符宽度。

`compute_selection_highlight_quads`（render_geom.rs:49）调用链：
```
local_clip_start = clip_start - line_abs           // = vl_byte_start (第二条起)
x_start = byte_to_x(local_clip_start, clusters, …) // ❌ 错算
```

旁证：`render_pipeline.rs:622` 在另一处计算 cursor x 时 *专门* 做了 `c.byte_range.end - vl_byte_start` 归零——足以说明此处约定混乱。

### 修复方案：方案 B（统一改为 vl-local 字节坐标）

> 已选定方案 B：把 `AdvanceCacheEntry.clusters` 中的 `cluster_end_byte` 从「整 doc-line 字节偏移」改为「相对 vl 起点的字节偏移」（vl-local）。这样 `byte_to_x` 的 `prev_end=0` 初值天然正确，且**与 `vl_byte_start` 字段语义完全一致**（vl_byte_start 本来就是 line-local 的）——结构上更清晰，杜绝两种坐标混用的二义性。

#### 4.B 步骤一：统一约定（写进 doc 注释）

文件：`crates/ui/src/render_geom.rs::AdvanceCacheEntry`（行 9–13）

在结构体定义上方加注释：

```text
/// Per-visual-line data for hit-testing, selection rendering, and cursor movement.
///
/// 字节坐标语义（统一约定）：
/// - `vl_byte_start`: 该 vl 起点 *相对 doc line 起点* 的字节偏移（line-local）。
/// - `clusters[i].0`（cluster_end_byte）: 该 cluster 末尾 *相对 vl 起点* 的字节偏移（vl-local）。
///
/// 即对于第 i 个 cluster：
///   cluster 在 doc 中的绝对字节范围 = [line_byte_offset(doc_line) + vl_byte_start + prev.0,
///                                       line_byte_offset(doc_line) + vl_byte_start + clusters[i].0)
```

#### 4.B 步骤二：生产端只改一处

文件：`crates/ui/src/layout.rs::build_advance_cache_entries`（行 6–38）

行 22–28 现状：
```rust
for c in &shaped.clusters[vl_start..vl_end] {
    ...
    cluster_advances.push((c.byte_range.end, x));  // ← integer line-local
}
let vl_byte_start = shaped.clusters[vl_start].byte_range.start;
```

改为：
```rust
let vl_byte_start = shaped.clusters[vl_start].byte_range.start;
for c in &shaped.clusters[vl_start..vl_end] {
    ...
    cluster_advances.push((c.byte_range.end - vl_byte_start, x));  // ← vl-local
}
```

注意：`shaped.clusters[].byte_range` 在 shaper 内部是 line-local（已知约定），所以 `vl_byte_start` 与 `c.byte_range.end` 同坐标系，相减得到 vl-local 末尾偏移。

#### 4.B 步骤三：列出所有消费方并适配

通过 grep 全仓 `entry.clusters` / `clusters.last()` / `clusters[*]` 列出所有消费点。下表完整覆盖：

| # | 文件 / 函数                                                              | 行号        | 当前用法                                                                 | vl-local 后是否需改                            |
| - | ------------------------------------------------------------------------ | ----------- | ------------------------------------------------------------------------ | ---------------------------------------------- |
| 1 | `ui/src/render_geom.rs::byte_to_x`                                       | 21–42       | 输入 `byte_offset` 是 line-local；clusters 元素是 line-local             | **改：byte_offset 改为 vl-local**（见步骤四） |
| 2 | `ui/src/render_geom.rs::compute_selection_highlight_quads`               | 49–117      | 行 94–97：`local_clip_start = clip_start - line_abs`（line-local）；调 `byte_to_x` | **改**：传给 byte_to_x 之前再减 `vl_byte_start`，即 `local_clip_start - vl_byte_start` |
| 3 | `ui/src/decorations.rs::*`（搜索高亮入口）                               | 8–37 / 111+ | 调 `compute_selection_highlight_quads`                                  | 不动（call site 复用 #2 的修复）              |
| 4 | `ui/src/decorations.rs:128-132` (visible end 计算)                       | 128–132     | `last_entry.clusters.last().0` 用作"visible 字节末尾偏移"                | **改**：`last_line_abs + vl_byte_start + last_cluster_end`（加上 vl_byte_start） |
| 5 | `app/src/commands.rs::cursor_visual_line_bounds`                         | 16–43       | 行 28：`entry.clusters.last().map(|&(e, _)| e)` 当 line-local end       | **改**：`entry.vl_byte_start + e`              |
| 6 | `app/src/commands.rs::cursor_visual_line_bounds`                         | 41–42       | 行 41：`vl_end = entry.clusters.last().map(...).unwrap_or(vl_start)`；返回 `(line_abs_start + vl_start, line_abs_start + vl_end)` | **改**：`vl_end = entry.vl_byte_start + e`，下一行同理 |
| 7 | `app/src/commands.rs::home_visual_line_bounds`                           | 49–71       | 行 66：同上                                                              | **改**：同 #5                                  |
| 8 | `app/src/render_pipeline.rs` cache 命中路径（cursor 定位）              | 285–295     | 行 289 `clusters_for_vl.push((cd.1, px))` 已经把 raw cluster_end 写入新表；行 291 `vl_byte_start = cached.cluster_data.get(vl_start).map(|cd| cd.0).unwrap_or(0)` | **不动**：这里写的是 RenderCache 内部表，与 `AdvanceCacheEntry` 解耦——但需要核对 RenderCache 的 cluster_data 也按 vl-local 存（见步骤五） |
| 9 | `app/src/render_pipeline.rs` 非 cache 路径（cursor x 计算）              | 614–624     | 行 614 `vl_byte_start = shaped.clusters[vl_start].byte_range.start`；行 622 `cluster_xs.push((c.byte_range.end - vl_byte_start, x))` | **不变**：已经是 vl-local（说明本次修复正是把别处对齐到这里）|
| 10 | `app/src/render_pipeline.rs::shape_visible_lines` 推 advance_cache 入口 | 180、217、409、495 | `clusters: Vec::new()` 空 entry                                | 不动                                            |
| 11 | `app/src/commands.rs` 单测调用点                                         | 755、795、1018、1038、1051、1064 等 | 测试构造 `AdvanceCacheEntry { vl_byte_start: 5, clusters: vec![(10, ...)] }` | **改**：测试中 cluster_end 改为 vl-local（如 `vl_byte_start: 5, clusters: vec![(5, ...)]` 表示同一含义）。共 ~10 处单测，机械替换 |
| 12 | `ui/src/render_geom.rs` 自身单测                                          | 124–156     | 测试 `byte_to_x` 直接传 line-local 偏移                                  | **改**：测试期望按新签名传 vl-local 偏移      |

#### 4.B 步骤四：`byte_to_x` 输入语义统一

文件：`crates/ui/src/render_geom.rs::byte_to_x`（行 21–42）

签名不动，但 docstring 改为：

```text
/// Map a byte offset (relative to the visual-line start) to its pixel x position.
///
/// `clusters`: sorted `(cluster_end_byte_vl_local, pixel_x)` pairs.
/// `byte_offset`: vl-local byte offset.
```

**实现完全不动**——`prev_end = 0` 现在恰好对应 vl 起点，是正确的初值。

#### 4.B 步骤五：核查 RenderCache 内部表

文件：`crates/app/src/render_pipeline.rs:285–295`

```rust
let mut clusters_for_vl = Vec::new();
for cd in &cached.cluster_data[vl_start..vl_end] {
    let px = cd.2; // pixel_x
    clusters_for_vl.push((cd.1, px)); // ← cd.1 是 cluster_end_byte
}
let vl_byte_start = cached.cluster_data.get(vl_start).map(|cd| cd.0).unwrap_or(0);
advance_cache.push(AdvanceCacheEntry {
    doc_line: doc_line_idx,
    vl_byte_start,
    clusters: clusters_for_vl,
});
```

需要确认 `cached.cluster_data[*].1`（cluster end）是 line-local 还是 vl-local。如果是 line-local（与 shaped.clusters 同源），需在此处改为 `cd.1 - vl_byte_start`，与 `build_advance_cache_entries` 对称。**待复核 `cluster_data` 写入端**（grep `cluster_data` 在 RenderCache 内部）。

| 复核子项 | 文件                           | 期望                              |
| -------- | ------------------------------ | --------------------------------- |
| 写入端   | RenderCache 内部填充 `cluster_data` 的位置 | 写入是 line-local（与 shaper 一致） |
| 读取端   | `render_pipeline.rs:285-295`   | 读出后减 `vl_byte_start` 再 push   |

如果写入端已经是 vl-local，则此处不动；如果是 line-local，按上述减一遍。

#### 4.B 步骤六：分批提交

| 批次 | 范围                                                                          | 可独立编译？ |
| ---- | ----------------------------------------------------------------------------- | ------------ |
| B-1  | 步骤一（注释）                                                                | ✅           |
| B-2  | 步骤二（生产端 `build_advance_cache_entries`）+ 步骤三 #2/#5/#6/#7（消费端线上代码） | ❌ 必须一起 |
| B-3  | 步骤三 #11 + #12（单测构造调整）                                              | ❌ 与 B-2 一起 |
| B-4  | 步骤四（docstring）+ 步骤五（RenderCache 复核）                              | ✅           |

实际操作合并 B-2 + B-3 在同一 commit；B-1 与 B-4 可前后分开。

### 影响 / 测试

- 既有 `byte_to_x_first_cluster`、`byte_to_x_exact_boundary`、`byte_to_x_past_end`、`byte_to_x_empty_clusters`（render_geom.rs 单测）：测试输入需改为 vl-local（机械替换）。
- 既有 `commands.rs` 中 ~10 处 `AdvanceCacheEntry { vl_byte_start: N, clusters: vec![(M, ...)] }` 测试 fixture：把 `M` 改为 `M - N`（对那些 vl_byte_start > 0 的）。
- **新增**单测：
  - `compute_selection_highlight_quads_multi_vl_left_aligned`：构造 ≥2 vl 选区，断言第 2 vl 的 quad 左边界 == left_margin。
  - `byte_to_x_vl_local_at_zero`：`byte_offset=0`、`clusters=[(N, X)]` → 返回 `left_margin`（不依赖 vl_byte_start）。
  - `cursor_visual_line_bounds_with_nonzero_vl_start`：构造 `vl_byte_start=10, clusters=[(5, ...)]`（vl-local 5 字节）→ 返回 `(line_abs+10, line_abs+15)`。

### 边界情况

- 第一条 vl（`vl_byte_start = 0`）：vl-local == line-local，行为完全等价旧代码。
- 跨多个 doc 行：每个 doc line 单独 shape，每条 vl 都按各自的 `vl_byte_start` 起算——没有跨 line 的字节比较，安全。
- 选区右端：`byte_to_x(local_clip_end - vl_byte_start, …, is_end=true)` 也走同一签名，对称无遗漏。
- 搜索高亮（decorations.rs:111+）：通过 `compute_selection_highlight_quads` 复用，自动覆盖。
- cursor 渲染：非 cache 路径已是 vl-local（步骤三 #9）；cache 路径取决于步骤五——必须复核。

### 风险与回滚

- 改动面：1 个生产端 + 6 个消费点 + ~12 处测试 fixture，约 30–50 行。
- 单一坐标系语义改变，**所有消费点必须同步改**，否则编译过但行为错。每个消费点都有单测兜底，回归风险可控。
- 回滚：`git revert` 单 commit（B-2/B-3 合并后），结构整洁。

### 影响 / 测试

- 现有 `compute_selection_highlight_quads` 测试基本覆盖单 vl 场景，需新增多 vl 场景：构造 `advance_cache` 含 ≥2 个软换行 vl，跨 vl 选区，断言第 2 vl 的 x_start ≈ left_margin。
- 同步检查搜索高亮（`search_match` 路径）是否也受影响——理论上同因，应一并修。
- 硬换行情形 `vl_byte_start = 0`，行为等价于原代码，零回归。

### 边界情况

- 第一条 vl（`vl_byte_start = 0`）：`prev_end = 0`，行为等价旧代码。
- 选区右端在 vl 内部：`x_end` 算法对称错误也会出现吗？查 `is_end=true` 分支：
  ```rust
  if c_end >= byte_offset { ... }
  ```
  同样的 `prev_end=0` 偏差也会影响 x_end，但因为 byte_offset 通常落在 cluster *中部或末尾*，错误幅度更小，但仍不为零。修复时**两端**都用新参数初始化。

---

## 阶段 5 — B5：IME 输入未显示已输入字母

### 现象
在 macOS 输入英文字母时，看不到 IME preedit 阶段的下划线字符（应在光标处临时显示）。中文输入也可能出现"先看到字母 n、再看到候选字"的双插入现象。

### 根因
文件：
- `crates/app/src/app_lifecycle.rs:138-147` IME 事件已正确写入 `self.preedit_text`、`self.preedit_cursor`，并设 `needs_redraw = true`。
- `crates/app/src/app.rs:138-140` 状态字段存在。
- `crates/app/src/app_renderer.rs:475-509` 渲染层已消费 preedit。
- **真正问题在 `app_lifecycle.rs:209` 的 `WindowEvent::KeyboardInput` 分支**：

```rust
WindowEvent::KeyboardInput { event, .. } if event.state.is_pressed() => {
    let actions = crate::events::handle_keyboard(self, &event);   // ❌ 无 IME 守卫
    ...
}
```

这里**没有任何 IME 状态守卫**。在 macOS + winit 0.30 下，按字母键时 winit 同时派发：
1. `Ime::Preedit("n", ...)` → 存入 `preedit_text`，渲染会画
2. `WindowEvent::KeyboardInput { logical_key: Character("n"), text: Some("n") }` → `key_to_command` → `EditCommand::InsertChar("n")` → **直接写入文档**

结果：
- `n` 立即被作为正式字符插入到 buffer，触发 reshape + cache invalidate；
- preedit 还没机会被画出来；
- 后续 `Ime::Commit` 再次插入一遍 → 双插入或乱码。
- 即便 preedit 阶段成功画出，下一个按键又会走"先 InsertChar 再 Preedit("")" 的竞速，看起来"preedit 一闪就没"。

辅助证据：
- `app.rs:2698-2705` 的测试 `test_preedit_empty_string_with_cursor` 已经显式承认 winit 会发空 preedit。
- `Ime::Commit` handler（行 149）已经做了完整的字符插入逻辑——KeyboardInput 路径不应再走插入。

### 修复方案

在 `app_lifecycle.rs:209` 的 KeyboardInput 分支前加 **IME 互斥守卫**。

#### 守卫条件

满足 *任一* 条件视为 "IME 正在 composition / 处理键盘"，跳过 `key_to_command` 中插入字符的命令（但保留导航 / 快捷键命令）：

1. `event.logical_key == Key::Named(NamedKey::Process)` — 平台明确说"我吃了这个键"。
2. `!self.preedit_text.is_empty()` — 当前正处于 IME composition。
3.（可选）`event.text` 在最近一次 `Ime::Preedit` 事件后立刻到达——通过帧序号或时间戳判定。

#### 实现策略

两层过滤：

- **底层**：`events::handle_keyboard` 内部判断到上述条件时，直接 return 空 actions。
- **同时**：`input::key_to_command` 对 `EditCommand::InsertChar` 类命令再加一道兜底（防御性）。

让 IME `Commit` 走唯一插入路径（行 149-207 已实现）；KeyboardInput 仅处理快捷键 / 导航。

### 影响 / 测试

- 既有的 IME 测试（`test_preedit_empty_string_with_cursor` 等）保持通过。
- **必须新增**端到端测试：模拟 winit 同时派发 `Ime::Preedit("n", ...)` + `KeyboardInput(Character("n"))`，断言文档内容**未**插入 'n'，仅 `preedit_text == "n"`。
- 模拟 `Ime::Commit("中")` 后断言文档插入"中"且 preedit 清空。
- 验证非 IME 场景的快捷键（如 Cmd+S, 方向键）仍工作。

### 边界情况

- **macOS Dead Key**（如长按 e 出 é）：会发 `Ime::Preedit("´", ...)` 然后 `Ime::Commit("é")`——KeyboardInput 也会发字母键，必须被守卫吃掉。
- **Linux IBus / Windows IME**：行为可能不同，winit 通常会用 `Key::Named(NamedKey::Process)` 标记，第 1 条守卫覆盖。
- **快捷键场景**（Cmd+C 等）：`logical_key` 不是 `Process`、`preedit_text` 为空——守卫不触发，正常处理。
- **空 preedit (`Ime::Preedit("", None)`)**：清空 preedit_text，下一个 KeyboardInput 视为正常输入——这是 IME composition 结束后的正常输入，期望行为。

---

## 阶段汇总：交付顺序与依赖

| 阶段 | 改动文件                               | 是否依赖前序 | 估算改动行数 |
| ---- | -------------------------------------- | ------------ | ------------ |
| 1    | `app/src/ui_shell.rs`、`app/src/app.rs` | 无          | ~30（顺序调整 + 新增字段 + spacer widget） |
| 2    | `app/src/events.rs`、`app/src/app.rs`、`ui/src/widgets/scrollbar.rs`、`ui/src/scrollbar.rs` | 无 | ~30（redraw + 粗细切换） |
| 3    | `app/src/render_pipeline.rs`           | 无           | ~15          |
| 4    | `ui/src/render_geom.rs`、`ui/src/layout.rs`、`ui/src/decorations.rs`、`app/src/commands.rs`（含测试） | 无 | ~50（含测试 fixture 调整） |
| 5    | `app/src/app_lifecycle.rs`、`events.rs` | 无          | ~20          |

各阶段独立，可任意顺序，建议按 **1 → 2 → 5 → 3 → 4** 推进（先解决最显眼的视觉/交互问题，再处理光标精度，最后处理选择精度）。

每个阶段提交前必须：
- 编译通过（`cargo check`）
- 现有相关单测通过
- 新增单测覆盖该 Bug 的最小复现

---

## 风险与回滚

| 阶段 | 主要风险                                       | 回滚成本   |
| ---- | ---------------------------------------------- | ---------- |
| 1    | 极低；只调换 push 顺序                         | revert 1 文件 |
| 2    | 低；仅多触发 redraw，可能略增帧率              | revert 2 文件 |
| 3    | 中；归属规则切换可能影响极端边界场景           | revert 1 文件，加单测兜底 |
| 4    | 中；`byte_to_x` 签名变更，所有调用方需适配     | revert 多文件，建议先 grep 确认调用面 |
| 5    | 中高；IME 守卫不当会导致快捷键失灵或字符无法输入；多平台差异 | 留功能开关或快速 revert |

阶段 5 建议在合并前做**多平台手动测试**（macOS 中文/英文、Linux IBus 中文、Windows MSIME），并在 `docs/manual_test_protocol.md` 增补 IME 用例。
