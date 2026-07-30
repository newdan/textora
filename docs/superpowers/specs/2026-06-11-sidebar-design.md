# Sidebar 与 Tab 双模式布局设计

- 日期：2026-06-11
- 状态：待实施
- 范围：新增侧边栏（参考 codex / Zed），与现有 Tab 栏互斥；菜单组件抽离复用；tab_bar 与 sidebar 对称内聚化。

## 1. 目标与非目标

### 目标

- 新增 **Sidebar 模式**：左侧常驻一列，包含 macOS 红绿灯按钮、☰ 切换、新建、已打开文档列表、设置按钮。
- 与现有 **Tabs 模式** 互斥：同一时刻只能看到其中一种顶部容器，由全局设置决定。
- Sidebar 支持 **钉住 / hover 弹出 / 边缘拖拽改宽**。
- 借此机会把 `tab_bar` 内聚化（与 sidebar 对称），并把通用菜单组件 `popup_menu` 抽离。

### 非目标（明确排除）

- 文件树 / 项目浏览 / git 状态 / 大纲 / 项目搜索（v2 阶段再说）
- 多 sidebar（左右两侧）
- Linux / Windows 的原生 titlebar 整合（首发只做 macOS）
- 自定义图标包 / 主题扩展
- 实时监听 `settings.toml` 外部变更
- 设置面板的图形化控件（toggle / slider 等），本期设置入口走简单菜单 + 直接打开 settings.toml 文件

## 2. 用户体验

### 2.1 默认行为

- 首次启动：`view_mode = Sidebar`，`pinned = false`，`width = 220 * dpi_scale`。
- 红绿灯按钮整合进 sidebar 顶部 28px header（macOS NSWindow `fullSizeContentView`）。
- 编辑区左侧默认留白 `dpi_scale * 10`（独立于 sidebar 宽度），让 hover 热区视觉更宽松。

### 2.2 状态机

侧边栏 visibility 三态：

| 状态 | 触发 | 编辑区表现 |
|---|---|---|
| `Hidden` | 默认（未钉住、鼠标不在热区） | 无 sidebar，仅顶部 header（含红绿灯 + ☰） |
| `HoverPeek` | 鼠标停留窗口最左 4px 热区 ≥ 150ms | overlay 浮层，编辑区不让位 |
| `Pinned` | Cmd+B 或 ☰ 点击切换 | 编辑区水平让位 `width` 像素 |

转移规则：

- `Hidden → HoverPeek`：鼠标进入最左 4px 热区，停留 150ms。
- `HoverPeek → Hidden`：鼠标离开 sidebar 区域 300ms 后；或按 Esc 立即收起。
- `Hidden ↔ Pinned`：Cmd+B 或 ☰ 点击。
- `HoverPeek → Pinned`：在 hover 出现后点击 ☰ 或按 Cmd+B。
- 极窄窗口（窗口宽 < `width + 100px`）：强制 `Hidden`，禁止 Pinned。

### 2.3 模式切换

`view_mode` 由设置菜单或编辑 settings.toml 修改：

- `Sidebar → Tabs`：sidebar 状态丢弃；NSWindow 还原原生 titlebar；Tab 栏出现；编辑区不需要重新布局视口左边距（默认 `dpi_scale * 10`）。
- `Tabs → Sidebar`：Tab 栏状态丢弃；NSWindow 改为 `fullSizeContentView`；sidebar 进入 `Hidden` 初态；红绿灯浮在 sidebar header。

不需要重启 app；切换后 `request_redraw` 一次。

### 2.4 键盘

- `Cmd+B`：仅在 Sidebar 模式生效，切换 Pinned ↔ Hidden（hover overlay 视为 Hidden）。
- `Esc`：仅在 HoverPeek 状态生效，立即收起。
- 其他全局快捷键不受影响。

## 3. 架构

### 3.1 模块切分

```
crates/ui/src/
├── view_mode.rs      // 新增：pub enum ViewMode { Sidebar, Tabs }
├── popup_menu.rs     // 新增：从 tab_bar.rs 抽离 PopupMenu / PopupMenuItem / PopupMenuAction
│                     //       / popup_menu_vertices / popup_menu_text_positions / hit_test
├── tab_bar.rs        // 重构：散装函数 → TabBarState 内聚（见 3.3）
├── sidebar.rs        // 新增：与 tab_bar 对称内聚
└── lib.rs            // 导出新模块

crates/app/src/
├── sys/
│   └── macos_titlebar.rs   // 新增：NSWindow titlebar 隐藏 + fullSizeContentView
│                            // 非 macOS 编译为 no-op stub
└── workspace.rs       // 持有 tab_bar_state + sidebar_state，每帧二选一驱动
```

依赖方向不变：`app → ui → render → shaping → core`。`sidebar` 不引入新外部依赖；`macos_titlebar` 通过 `objc2` 或现有 `objc` crate 调用 AppKit（具体选型在实施阶段评估，优先复用项目已引入的）。

### 3.2 通用菜单组件 `ui::popup_menu`

从 `tab_bar.rs:962-1300+` 抽离：

- `PopupMenu { items, item_rects, menu_rect }`
- `PopupMenuItem { label, action, is_active, is_separator }`
- `pub enum PopupMenuAction` 改为泛型容器：保留 `SwitchTab(usize)` / `Context(ContextMenuAction, usize)`，新增 `Custom(u32)` 用于 sidebar 设置菜单选项编码（避免菜单组件与业务耦合）。
- `popup_menu_vertices` / `popup_menu_text_positions` / `PopupMenu::hit_test` 直接复用。
- `tab_bar.rs` 改为 `use ui::popup_menu::*;` 并 re-export 旧符号路径，避免 app 层连锁修改。

复用方：

- tab_bar：overflow 下拉、tab 右键菜单
- sidebar：设置按钮下拉、文件项右键菜单

### 3.3 tab_bar 内聚化

引入 `TabBarState` 收口现有散落状态：

```rust
pub struct TabBarInput<'a> {
    pub tabs: &'a [TabInfo],
    pub active_index: Option<usize>,
    pub pinned_indices: &'a HashSet<usize>,
    pub back_enabled: bool,
    pub forward_enabled: bool,
    pub screen_w: f32,
    pub screen_h: f32,
}

pub struct TabBarState {
    layout: Option<TabBarLayout>,
    scroll_offset: f32,
    hovered_index: Option<usize>,
    preview_index: Option<usize>,
    drag: Option<TabDragState>,
    open_menu: Option<PopupMenu>,
}

impl TabBarState {
    pub fn new() -> Self;
    pub fn update_layout(&mut self, input: &TabBarInput, shaper: Option<&mut Shaper>);

    pub fn on_mouse_move(&mut self, x: f32, y: f32);
    pub fn on_mouse_leave(&mut self);
    pub fn on_click(&mut self, x: f32, y: f32, button: MouseButton) -> Option<TabBarAction>;
    pub fn on_scroll(&mut self, dx: f32);
    pub fn on_drag(&mut self, x: f32, y: f32) -> Option<TabBarAction>;
    pub fn on_drag_end(&mut self) -> Option<TabBarAction>;

    pub fn vertices(&self, theme: &Theme) -> Vec<GlyphVertex>;
    pub fn text_positions(&self, font_size: f32) -> Vec<TextPosition>;
    pub fn current_layout(&self) -> Option<&TabBarLayout>;
}

pub enum TabBarAction {
    SwitchTab(usize),
    CloseTab(usize),
    NewEmptyTab,
    NavigateBack,
    NavigateForward,
    ReorderTabs { from: usize, to: usize },
    OpenContextMenu { tab_index: usize, anchor: [f32; 2] },
    OpenOverflowMenu,
    Context(ContextMenuAction, usize),
}
```

迁出 app 层的状态：`workspace.tab_layout` / `workspace.hovered_tab_index` / `workspace.preview_tab_index` / `scroll_offset` 全部归入 `TabBarState`。`workspace` 仍持有这一个对象，但只通过方法访问。

兼容路径：旧的散装函数（`layout_tabs` / `hit_test` / `tab_bar_vertices` / `set_preview_tab`）保留为 `pub(crate)` 实现细节，仅供 `TabBarState` 内部调用，不再对 app 暴露。

### 3.4 sidebar 内聚封装

```rust
pub struct SidebarInput<'a> {
    pub tabs: &'a [TabInfo],
    pub active_index: Option<usize>,
    pub screen_w: f32,
    pub screen_h: f32,
}

pub struct SidebarConfig {
    pub pinned: bool,
    pub width: f32,
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum Visibility { Hidden, HoverPeek, Pinned }

pub struct SidebarState {
    visibility: Visibility,
    hover_enter_at: Option<Instant>,
    hover_leave_at: Option<Instant>,
    drag: Option<EdgeDragState>,
    layout: Option<SidebarLayout>,
    open_menu: Option<PopupMenu>,
}

impl SidebarState {
    pub fn new(cfg: &SidebarConfig) -> Self;
    pub fn update_layout(&mut self, input: &SidebarInput, cfg: &SidebarConfig);

    pub fn on_mouse_move(&mut self, x: f32, y: f32, screen_w: f32, screen_h: f32);
    pub fn on_mouse_leave(&mut self);
    pub fn on_click(&mut self, x: f32, y: f32, button: MouseButton) -> Option<SidebarAction>;
    pub fn on_drag(&mut self, x: f32, y: f32) -> Option<SidebarAction>;
    pub fn on_drag_end(&mut self) -> Option<SidebarAction>;
    pub fn on_key(&mut self, key: SidebarKey) -> Option<SidebarAction>;
    pub fn tick(&mut self, now: Instant);

    pub fn vertices(&self, theme: &Theme) -> Vec<GlyphVertex>;
    pub fn text_positions(&self, font_size: f32) -> Vec<TextPosition>;
    pub fn current_width(&self, cfg: &SidebarConfig) -> f32;
    pub fn editor_left_offset(&self, cfg: &SidebarConfig) -> f32;
    pub fn is_visible(&self) -> bool;
}

pub enum SidebarAction {
    SwitchTab(usize),
    NewDocument,
    OpenSettingsMenu,
    ToggleViewMode,
    TogglePin,
    SetWidth(f32),
    Context(ContextMenuAction, usize),
}

pub enum SidebarKey { TogglePin, Escape }
```

内聚边界（app 层不再处理这些细节）：

| 行为 | 在 sidebar 内 | app 层只做 |
|---|---|---|
| 4px 热区进入 + 150ms 延时 | `on_mouse_move` + `tick` | 转发坐标 |
| 离开 300ms 自动收起 | `tick` | 每帧调一次 tick |
| Cmd+B / Esc | `on_key` | 转发 |
| 边缘拖拽改宽 | `on_drag` + `EdgeDragState` | 转发坐标 |
| 命中按钮 / 文件项 / 设置 / ☰ | `on_click → SidebarAction` | 执行 action |
| 设置菜单 / 文件项右键菜单 | 内部持有 `Option<PopupMenu>` | 不感知 |

### 3.5 互斥渲染入口

`render_pipeline.rs` 顶层：

```rust
let view_mode = Settings::get().view_mode;
match view_mode {
    ViewMode::Tabs => {
        workspace.tab_bar_state.update_layout(&tab_input, shaper);
        let editor_left = settings.dpi_scale * 10.0;
        // ... render tab bar + editor
    }
    ViewMode::Sidebar => {
        workspace.sidebar_state.update_layout(&sb_input, &cfg);
        workspace.sidebar_state.tick(Instant::now());
        let editor_left = workspace.sidebar_state.editor_left_offset(&cfg)
                          + settings.dpi_scale * 10.0;
        // ... render sidebar + editor
    }
}
```

事件分发同样按 `view_mode` 二选一调用 `tab_bar_state` / `sidebar_state` 的 `on_*`。

## 4. 数据流与持久化

### 4.1 配置层级

| 字段 | 持久化位置 | 类型 | 默认值 | 加载时机 |
|---|---|---|---|---|
| `view_mode` | `~/.edit+/settings.toml` | `ViewMode` | `Sidebar` | 启动 |
| `sidebar_pinned` | `~/.edit+/workspace.yaml` | `bool` | `false` | workspace 加载 |
| `sidebar_width` | `~/.edit+/workspace.yaml` | `f32`（px，含 dpi_scale） | `220 * dpi_scale` | workspace 加载 |

约束：

- `sidebar_width` 加载时强制 `clamp(160 * dpi_scale, 400 * dpi_scale)`。
- 字段缺失走默认值，不报错不阻塞启动。

### 4.2 写盘策略

| 触发 | 写盘位置 | 时机 |
|---|---|---|
| 设置菜单切 view_mode | settings.toml | 立即 |
| Cmd+B / 设置菜单切 pin | workspace.yaml | 立即 |
| 边缘拖拽改 width | workspace.yaml | `on_drag_end` 一次 |

拖拽过程中只改内存，避免高频 IO。

### 4.3 每帧数据流

```
[workspace.doc_views] → TabInfo[]
                            │
            ┌───────────────┴───────────────┐
            ▼                               ▼
   ViewMode::Tabs                   ViewMode::Sidebar
   tab_bar_state.update_layout      sidebar_state.update_layout(cfg)
   tab_bar_state.vertices/text      sidebar_state.tick + vertices/text
            │                               │
            └───────────────┬───────────────┘
                            ▼
                render_pipeline 提交 GPU
                            │
                            ▼
        editor_left_offset → DocumentView 视口左边距
        （Sidebar Pinned 让位 width；HoverPeek/Hidden 为 0；
         Tabs 恒为 dpi_scale * 10）
```

### 4.4 macOS titlebar 集成

仅 `ViewMode::Sidebar` 激活：

- 进入 Sidebar：`sys::macos_titlebar::enable_full_size_content(window)`
  → `titlebarAppearsTransparent = true`
  → `styleMask |= NSWindowStyleMask::FullSizeContentView`
  → `titleVisibility = NSWindowTitleVisibility::Hidden`
- 进入 Tabs：`disable_full_size_content(window)` 还原。
- 查询红绿灯占用：`traffic_light_inset(window) -> (left_px, top_px)`，sidebar header 顶部 padding ≥ 该值。
- 非 macOS：所有函数 stub 返回 `(0.0, 0.0)` 或 no-op。Sidebar 模式在 Linux/Windows 用普通窗口边框，不在本期范围内深度优化。

## 5. 错误处理与边界

### 5.1 错误处理

- objc 调用失败（NSWindow 拿不到、selector 不存在）：log error，回退到 inset `(0, 0)`，不阻塞渲染。
- settings.toml / workspace.yaml 字段缺失：默认值。
- `width` 越界：clamp + warn log。
- DPI 切换：sidebar 跟随 `dpi_scale` 重算 width；状态保持。

### 5.2 边界场景

| 场景 | 期望行为 |
|---|---|
| 0 个 tab | sidebar 文件列表区显示空状态文案"无打开文档" |
| 100+ tab | 文件列表内部纵向滚动（复用 `ui::scrollbar`） |
| 极窄窗口（< width + 100px） | 强制 Hidden，禁止 Pinned |
| Pinned 状态切到 Tabs | sidebar 一帧内消失，下一帧 Tab 栏出现 |
| HoverPeek 状态切到 Tabs | 立即收起，切 Tab |
| 拖拽 width 时 view_mode 被外部改 | 取消拖拽，丢弃临时宽度 |
| 拖拽 width 时鼠标离开窗口 | 沿用 winit capture，松开正常 commit |
| Cmd+B 在 Tabs 模式 | no-op |
| settings.toml 被外部改 | 本期不监听，下次启动生效 |
| 全屏 / Stage Manager 切换 | 重算 traffic_light_inset（监听 resize / occluded） |
| 文件项 ZWJ emoji 名 | 走 cosmic-text，与 tab 标题一致 |
| 文件名含 `\n` | sanitize 为空格 |
| 红绿灯被 sidebar 内容遮挡 | header 顶部 padding ≥ inset.1 |

## 6. 测试

### 6.1 ui::popup_menu

- `popup_menu_hit_test_basic`
- `popup_menu_hit_test_separator_skipped`
- `popup_menu_overflow_layout`（搬自 tab_bar）
- `popup_menu_context_layout`（搬自 tab_bar）

### 6.2 ui::tab_bar（重构后回归）

- 现有 tests 全绿，命名保留。
- 新增：
  - `tab_bar_state_hover_transition`
  - `tab_bar_state_scroll_clamp`
  - `tab_bar_state_drag_reorder`

### 6.3 ui::sidebar

- `sidebar_hover_enter_after_150ms`
- `sidebar_hover_exit_after_300ms`
- `sidebar_pinned_overlay_immune_to_leave`
- `sidebar_width_drag_clamp`
- `sidebar_width_drag_persists_only_on_drag_end`
- `sidebar_click_file_emits_switch_tab`
- `sidebar_settings_menu_open_close`
- `sidebar_empty_tabs_shows_placeholder`
- `sidebar_view_mode_toggle_collapses_overlay`
- `sidebar_extreme_narrow_window_disables_pin`

### 6.4 集成测试（headless smoke）

- `view_mode_switch_no_panic`：Sidebar ↔ Tabs 来回 10 次。
- `sidebar_persistence_roundtrip`：pin/width 写入 workspace.yaml 后加载一致。
- `macos_titlebar_apply_then_revert`：仅 macOS 编译，非 macOS 跳过。

### 6.5 手动验证（追加到 `docs/manual_test_protocol.md`）

1. 默认启动是 Sidebar 模式，红绿灯位于 sidebar header。
2. Cmd+B 切钉住，编辑区水平让位 / 恢复。
3. 鼠标贴左 < 4px 停 150ms → overlay 出现；离开 300ms → 消失。
4. 拖右边缘改宽，松手后重启 app 宽度保持。
5. 设置菜单 → 切 Tabs → 红绿灯回原生位、Tab 栏出现。
6. 100+ tab 时 sidebar 列表纵向滚动顺滑。
7. 全屏 / Stage Manager 切换不破坏布局。

## 7. 实施阶段切分（粗）

详细计划由 writing-plans 产出，这里只做粒度参考：

1. **抽离 popup_menu**（不引入功能变化，所有 tab_bar 测试保持绿）。
2. **tab_bar 内聚化为 TabBarState**（不引入功能变化，回归测试为主）。
3. **新增 ViewMode 枚举 + Settings/workspace 字段 + 持久化往返**。本阶段在交付时把内置默认临时设为 `ViewMode::Tabs`（因为 sidebar 实体还没实现）；待阶段 4 骨架就绪后再把默认值改为 `Sidebar`，与 §2.1 一致。
4. **新增 sidebar 模块骨架**（layout + 静态渲染 + 文件列表 click → SwitchTab）。本阶段强制 `Visibility::Pinned` 以便联调；hover 状态机和拖宽留给后续阶段。完成后把默认 view_mode 切回 `Sidebar`。
5. **macOS titlebar 桥接**（NSWindow fullSizeContentView，traffic_light_inset 查询）。
6. **hover 状态机 + 自动收起**。
7. **边缘拖拽改宽 + 持久化**。
8. **设置菜单（Sidebar/Tab 切换、打开 settings.toml）**。
9. **极窄窗口约束 + 边界场景修复 + 手动验证**。

每阶段独立可编译可测试，符合 CLAUDE.md §"实施计划"要求。

## 8. 风险与决策

| 风险 | 缓解 |
|---|---|
| winit 对 macOS titlebar customization 支持有限 | 直接通过 objc 调 NSWindow API；提前评估 `objc2` 与项目现有依赖兼容性 |
| `traffic_light_inset` 在不同 macOS 版本 / 全屏状态下值不同 | 监听 `WindowEvent::Resized` / `Occluded`；查询 NSWindow 的 `standardWindowButton(.closeButton).frame` |
| tab_bar 重构波及 app 层调用面大 | 旧散装函数保留为 `pub(crate)` 兼容；TabBarState 作为新外部入口；逐文件迁移 |
| 设置菜单重新发明 toggle 控件 | 本期严格走 PopupMenu，每个开关一个 item，避免引入控件库 |
| 高频拖宽抖动 | 仅 `on_drag_end` 写盘；拖拽期间只改 in-memory `cfg.width` |
| Sidebar 模式下编辑区视口左边距与 hover overlay 不一致导致跳变 | overlay 是 z-index 浮层，**不影响** `editor_left_offset`；只有 Pinned 时让位 |

## 9. 参考

- 现有代码：`crates/ui/src/tab_bar.rs`、`crates/ui/src/status_bar.rs`、`crates/app/src/workspace.rs`、`crates/app/src/render_pipeline.rs`、`crates/app/src/events.rs`
- 视觉参考：codex CLI、Zed、VSCode（折叠后窄条）
- 相关阶段：plans.md 阶段 9（多 buffer + Tab UI）已实现，本设计是阶段 9 的扩展
