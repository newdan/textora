# Edit+ 代码基清理与架构优化实施方案（修订版）

> 基于 `docs/project-audit-2026-06-14.md`、`docs/architecture_analysis.md` 以及逐项代码验证
> 日期：2026-06-14

---

## 验证总览

在撰写方案前，对两份审计文档的每项声明进行了逐行代码验证。结果：

| 验证结论 | 数量 | 说明 |
|---------|------|------|
| 确认属实 | 27 项 | 原审计正确 |
| **推翻** | 6 项 | 原审计错误（见下表） |
| 原审计未覆盖 | 14 项 | 新发现 |

### 被推翻的声明

| 原审计声明 | 实际情况 |
|-----------|---------|
| `traffic_light_inset` 未使用 | **被使用** — app.rs:2689 → ui_shell → sidebar 完整调用链 |
| `rebuild_and_layout` 未使用 | **被使用** — events.rs:482 |
| `ScrollbarAction` 从未构造 | **被使用** — scrollbar.rs 多处构造，app.rs 匹配处理 |
| `ui::core` 命名冲突 | **不存在** — `crate::core` 在 ui crate 内部指向自己的 foundation 模块，是 Rust 惯例 |
| 事件分发是硬编码 if-else | **不是** — Dock 已实现完整递归树派发（hit-test 路由、capture 支持、MouseMove 广播） |
| `steal_state`/`inject_state` 用于跨帧状态转移 | **实为死代码** — 生产代码只用轻量的 `inject_persistent`，`steal_state` 和 `inject_state` 仅测试用 |

---

## 新增发现：原审计未覆盖的问题

### A. 功能缺陷（影响用户）

| # | 发现 | 文件 | 影响 |
|---|------|------|------|
| A1 | `settings_io::save()` 从未调用 | `app/src/settings_io.rs:32` | 用户修改设置后重启丢失 |
| A2 | 导航历史完全未接入 | `workspace.rs:138-169` | `go_back`/`go_forward` 存在但零调用，标签页前进/后退功能缺失 |
| A3 | `RECENT_FILES` 从不填充 | `native_menu.rs:9` | 原生菜单"最近文件"永远为空 |
| A4 | `mouse::handle_cursor_moved` 是死代码 | `mouse.rs:95` | 鼠标拖拽选中可能通过其他路径工作，需确认 |

### B. 真正的性能/冗余问题

| # | 发现 | 文件 | 影响 |
|---|------|------|------|
| B1 | TabBarState 每帧计算两次 | `workspace.rs:743` + `tab_bar.rs:76` | workspace 和 widget 各自独立计算全量布局，结果存于两个不同实例 |
| B2 | 陈旧布局 bug | `workspace.rs:830` | `tick_scroll_animation` 推进偏移后不更新 `UiShell.tab_bar_state`，两套布局不同步 |
| B3 | `tab_infos` 每帧 clone 两次 | `app_renderer.rs:328,342` | 分别传给 set_tabs_input 和 set_sidebar_input |
| B4 | 6 处重复的 whitespace cluster advance 计算 | `render_pipeline.rs:578-579, 588-589, 660-661, 734-736, 804-805, 846-847` | 同一 `is_whitespace_cluster` + `ws_cluster_advance` 模式复制粘贴 6 次 |
| B5 | `visual_lines.clone()` 每可见行 | `render_pipeline.rs:576,586,879` | 1000 行文件约 1000 次小 Vec 分配 |
| B6 | `sidebar.rs:456, 979` 中 `item_h = 28.0 * dpi` 与 `ROW_H = 24.0` 不一致 | `sidebar.rs` | 值 28.0 应该用哪个常量不确定 |

### C. events.rs 遗留路径（架构债务）

| # | 发现 | 根因 |
|---|------|------|
| C1 | `handle_cursor_moved` 对 tab bar 做二次 dispatch + 手动 hit_test | `TabBarWidget::on_event` 对 MouseMove 错误地返回 `SwitchTab(idx)` 而非 `HoverTab`，迫使 events.rs 绕过 Dispatch 结果自己重做 |
| C2 | Sidebar hover 状态机有独立的并行输入路径 | `SidebarPersistent::on_mouse_move` 不在 Dock 派发链路内，必须在 events.rs:119 额外调用 |

### D. 标注错误

| # | 发现 | 文件 |
|---|------|------|
| D1 | `load_pinned()` 有 `#[allow(dead_code)]` 但实际被调用 | `workspace.rs:644` ← app.rs:1298 |
| D2 | `AppCommand` 有 `#[allow(dead_code)]` 但大量使用 | `menu_handler.rs:13` |

---

## Phase 0：快速胜利（预计 30 分钟）

与初版相同，无修改。

| # | 任务 | 操作 |
|---|------|------|
| 0.1 | 删除 `crash.log` 的 git 跟踪 | `git rm --cached crash.log` |
| 0.2 | 删除 `CLAUDE.md`（与 AGENTS.md 重复） | `git rm CLAUDE.md` |
| 0.3 | 修复 `ui/src/widgets/popup_menu.rs` 5 行重复 doc comment | 删 4 行，留 1 行 |

---

## Phase 1：死代码与警告清理（预计 3 小时）

> **重要：** 移除了原方案中 3 项被推翻的声明（traffic_light_inset、rebuild_and_layout、ScrollbarAction）。

### 1.1 删除确认无用的模块

| # | 文件 | 操作 |
|---|------|------|
| 1.1.1 | `core/src/terminal_stubs.rs` + `core/src/buffer/terminal_render.rs` | 删除文件。`terminal-render` feature 从未启用，terminal_stubs 的唯一消费者是未编译的 terminal_render.rs |
| 1.1.2 | `core/src/helpers.rs` 中的 `Size` 和 `Rect` 类型 | 删除。全项目零引用（`Point` 保留，在 commands.rs 和 document_view 中使用） |
| 1.1.3 | `stdext/src/alloc.rs`、`stdext/src/glob.rs` | 删除文件，更新 `stdext/src/lib.rs` 的 `mod` 声明 |
| 1.1.4 | 孤儿 `app/src/app_tests.rs` | 对比与 `render_pipeline_tests.rs` 的重复度。若测试不重复，在父模块中加 `#[cfg(test)] mod app_tests;`；抽取共用 `mock_cluster()` 到 `test_helpers.rs` |

### 1.2 删除未使用的函数/变量

| # | 文件 | 内容 |
|---|------|------|
| 1.2.1 | `app/src/app_renderer.rs` | 删除弃用的 `render_text_fragments`（零调用） |
| 1.2.2 | `app/src/app_renderer.rs:222` | 删除无意义的 `drop(lc)`（`usize` 是 Copy 类型） |
| 1.2.3 | `app/src/app_lifecycle.rs` | 删除重复的 `WINDOW_TITLE`（app.rs 已有） |
| 1.2.4 | `app/src/app_renderer.rs` | 删除未使用的 `use ui::tab_bar`、局部变量 `show_tabs`, `tbh`, `line_height` |
| 1.2.5 | `app/src/document_view/mod.rs` | 删除未使用的 `DisplayLineMap`、`RenderCache` import |
| 1.2.6 | `app/src/paint_backend.rs` | 删除未使用的 `is_whitespace_cluster` import |
| 1.2.7 | `app/src/reshape_worker.rs` | 删除赋值后从未读取的 `proxy` 变量 |
| 1.2.8 | `app/src/sidebar.rs` (widgets) | 删除死代码 `steal_state()` 和 `inject_state()`（仅测试用，生产只用 `inject_persistent`） |

### 1.3 修正错误的 `#[allow(dead_code)]`

| # | 文件 | 操作 |
|---|------|------|
| 1.3.1 | `app/src/workspace.rs:644` | 移除 `load_pinned()` 上的 `#[allow(dead_code)]`（实际在 app.rs:1298 被调用） |
| 1.3.2 | `app/src/menu_handler.rs:13` | 移除 `AppCommand` 上的 `#[allow(dead_code)]`（被大量使用） |
| 1.3.3 | `app/src/document_view/mod.rs:989` | 删除 `sync_after_edit_full()`（确认未被调用，只用增量方法） |

### 1.4 修复可见性问题

| # | 文件 | 操作 |
|---|------|------|
| 1.4.1 | `app/src/document_view/mod.rs` | `DisplayState` 和 `CursorState` 改为 `pub` 或暴露接口改为 `pub(crate)` |

---

## Phase 2：常量统一与硬编码消除（预计 3 小时）

> **修正：** `* 0.8` 有两个不同语义 —— 行号字号缩放（`LN_FONT_SCALE`）和基线偏移比例（`BASELINE_RATIO`），不能用同一个常量。

### 2.1 创建 `ui/src/constants.rs`

```rust
// === 尺寸 ===
pub const BAR_HEIGHT: f32 = 28.0;           // 统一 HEADER_H / SEARCH_BAR_HEIGHT / NEW_BTN_H / SETTINGS_BTN_H
pub const ROW_HEIGHT: f32 = 24.0;           // 列表行高
pub const SIDEBAR_DEFAULT_WIDTH: f32 = 220.0;
pub const SIDEBAR_MIN_WIDTH: f32 = 160.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 400.0;
pub const SCROLLBAR_THUMB_MIN_HEIGHT: f32 = 25.0;

// === 间距 ===
pub const H_PADDING: f32 = 12.0;
pub const MEDIUM_GAP: f32 = 10.0;
pub const SMALL_GAP: f32 = 8.0;
pub const TINY_GAP: f32 = 4.0;
pub const MICRO_GAP: f32 = 2.0;

// === 字体 ===
pub const BODY_FONT_SIZE: f32 = 14.0;
pub const TITLE_FONT_SIZE: f32 = 13.0;
pub const CAPTION_FONT_SIZE: f32 = 10.0;
pub const LN_FONT_SCALE: f32 = 0.8;         // 行号字号比（font_size * LN_FONT_SCALE）
pub const BASELINE_RATIO: f32 = 0.8;         // 基线位置比（line_height * BASELINE_RATIO）

// === 按钮/图标 ===
pub const BUTTON_SIZE: f32 = 16.0;
pub const CLOSE_BTN_SIZE: f32 = 12.0;

// === 排版 ===
pub const UNDERLINE_ALPHA: f32 = 0.75;
pub const TRAFFIC_LIGHT_TOTAL_W: f32 = 96.0; // 68 + 6 + 14 + 8
```

**注意：** 垂直居中比例（status_bar 0.35 / title_bar 0.6 / popup_menu 0.65）各有差异，因为不同字体基线偏移不同，**不强行统一**。各自保留为所在文件的 `const`。

### 2.2 逐文件替换

| # | 文件 | 替换内容 |
|---|------|----------|
| 2.2.1 | `ui/src/sidebar.rs` | `HEADER_H`/`NEW_BTN_H`/`SETTINGS_BTN_H` → `constants::BAR_HEIGHT`；间距值引用 constants |
| 2.2.2 | `ui/src/widgets/search_bar.rs` | `SEARCH_BAR_HEIGHT` → `constants::BAR_HEIGHT` |
| 2.2.3 | `ui/src/widgets/status_bar.rs` | 硬编码 32.0/10.0 → constants |
| 2.2.4 | `ui/src/widgets/title_bar.rs` | 硬编码 13.0/10.0 → constants |
| 2.2.5 | `ui/src/widgets/popup_menu.rs` | 硬编码 14.0/6.0/4.0 → constants |
| 2.2.6 | `app/src/render_pipeline.rs` | 4 处 `font_size() * 0.8` → `LN_FONT_SCALE`；5 处 `line_height * 0.8` → `BASELINE_RATIO` |
| 2.2.7 | `app/src/gutter.rs`、`decorations.rs`、`render_cache.rs` | `* 0.8` → `BASELINE_RATIO` 或 `LN_FONT_SCALE` |
| 2.2.8 | `app/src/app_renderer.rs` | `68.0 + 6.0 + 14.0 + 8.0` → `TRAFFIC_LIGHT_TOTAL_W` |
| 2.2.9 | `app/src/cursor_motion.rs` | 硬编码 32.0 → `constants::H_PADDING` |

### 2.3 统一 `WINDOW_TITLE`

- 保留 `app/src/app.rs` 中的定义，导出给 `app_lifecycle.rs` 使用
- 删除 `app_lifecycle.rs` 中的独立定义

### 2.4 统一 `test_theme()`（7 份 → 1 份）

| # | 任务 |
|---|------|
| 2.4.1 | 在 `ui/src/theme.rs` 中加 `#[cfg(test)] pub fn test_theme() -> Theme` |
| 2.4.2 | 各 widget 测试文件改为 `use crate::theme::test_theme;` |
| 2.4.3 | 删除 7 处本地 `fn test_theme()` 定义（合计 ~210 行） |

### 2.5 popup_menu 内联 27 色移入 Theme

`ui/src/widgets/popup_menu.rs` 中硬编码的 27 个 light theme 颜色 → 移入 `Theme::light()`，通过 `Settings::theme()` 获取。

---

## Phase 3：UI 双层架构合并（预计 2-3 天）

> **修正：** 移除了 Phase 3.1（重命名 `ui::core`），因为经验证不存在命名冲突。

### 3.1 Scrollbar 合并（先行试探）

旧模块只有 `compute_layout_px` + `ScrollbarLayoutPx` 纯函数。

| 步骤 | 操作 |
|------|------|
| 3.1.1 | 合并到 `ui/src/widgets/scrollbar.rs` |
| 3.1.2 | 旧文件改为 `pub use widgets::scrollbar::*;` 兼容 re-export |
| 3.1.3 | 更新外部引用，最终删除旧文件 |

### 3.2 StatusBar 合并

旧模块只有 `StatusBarInput` + `build_text`。

| 步骤 | 操作 |
|------|------|
| 3.2.1 | 合并到 `ui/src/widgets/status_bar.rs`，删除旧文件 |

### 3.3 TitleBar 合并

| 步骤 | 操作 |
|------|------|
| 3.3.1 | 合并 `ui/src/title_bar.rs` 到 `ui/src/widgets/title_bar.rs` |

### 3.4 PopupMenu 合并

旧模块有状态管理 + action 枚举。

| 步骤 | 操作 |
|------|------|
| 3.4.1 | 类型定义 → `widgets/popup_menu/types.rs` |
| 3.4.2 | 状态管理 → `widgets/popup_menu/state.rs` |
| 3.4.3 | 更新 `tab_bar/mod.rs` 中 popup_menu 的 re-export 链 |
| 3.4.4 | 删除旧文件 |

### 3.5 Sidebar 合并（最复杂，3054 行）

**目标结构：**
```
ui/src/widgets/sidebar/
├── mod.rs       # Widget trait 实现
├── state.rs     # SidebarState + SidebarAction
├── layout.rs    # SidebarLayoutItem 计算
├── paint.rs     # 绘制逻辑
├── types.rs     # SidebarInput, SidebarCfg 等
└── persistent.rs  # SidebarPersistent（保留跨帧持久状态）
```

| 步骤 | 操作 |
|------|------|
| 3.5.1 | 创建目录结构 |
| 3.5.2 | 迁移类型 → types.rs，状态 → state.rs，布局 → layout.rs |
| 3.5.3 | 整合两层 paint（widget 的动画层 + 旧模块的 chrome 绘制）→ paint.rs |
| 3.5.4 | 更新 app 层引用，删除旧 `ui/src/sidebar.rs` |

### Phase 3 验证

- 每步 `cargo check --all-targets` 零错误 + `cargo test` 全通过
- 手动测试：sidebar 展开/折叠、文件列表点击、右键菜单、tab 切换

---

## Phase 4：消除计算冗余 + events.rs 遗留路径清理（预计 2-3 天）

> **完全重写。** 原方案说"事件分发硬编码"是错误的——Dock 已是完整递归树。真正的问题是计算重复和 events.rs 中的 Workaround 代码。

### 4.1 消除 TabBar 双重布局（解决 B1 + B2）

**现状：** `workspace::update_tab_layout` 和 `TabBarWidget::set_tabs_input` 各自独立计算完整的 `TabBarLayout`，存于两个不同 `TabBarState` 实例。`tick_scroll_animation` 推进偏移后不更新 UiShell 的副本，造成陈旧布局 bug。

| 步骤 | 操作 |
|------|------|
| 4.1.1 | 移除 `UiShell.tab_bar_state` 字段 |
| 4.1.2 | app.rs 中需要读取布局的 3 个 action handler 改为从 Dock child 中获取 TabBarWidget 的布局 |
| 4.1.3 | auto-scroll 逻辑移入 TabBarWidget（或 widget 提供 `max_scroll()` 查询方法供 workspace 使用） |
| 4.1.4 | 让 `TabBarWidget::set_tabs_input` 成为唯一的布局计算入口 |

**改动量：** workspace.rs 行 711-808 的部分逻辑 + app.rs 行 961-1064 的读取方式 + ui_shell.rs 行 75 删除字段。

### 4.2 消除 events.rs 的 tab bar 二次 dispatch（解决 C1）

**根因：** `TabBarWidget::on_event` 对 `MouseMove` 返回 `SwitchTab(idx)` 而非 `HoverTab`，不正确的 action 迫使 events.rs 绕过 Dispatch 结果自己重做 hit_test。

| 步骤 | 操作 |
|------|------|
| 4.2.1 | `TabBarWidget::on_event` 对 `MouseMove` 改为返回 `None`（hover 状态通过内部 state 改变，不产生 action 给 app），或添加新 `WidgetAction::HoverTab(idx)` |
| 4.2.2 | `translate_widget_action` 中新增 `WidgetAction::HoverTab(idx)` → `AppAction::HoverTab(idx)` 映射 |
| 4.2.3 | 删除 events.rs:148-184 整段二次 dispatch + 手动 hit_test 代码 |

### 4.3 简化 Sidebar hover 状态机（解决 C2）

| 步骤 | 操作 |
|------|------|
| 4.3.1 | 让 `SidebarWidget::on_event` 处理后调用 `steal_persistent()` 将状态同步回 `UiShell.sidebar_persistent` |
| 4.3.2 | 删除 events.rs:115-120 的并行 `sidebar_on_mouse_move` 调用 |

### 4.4 Sidebar per-frame 分配优化

| 步骤 | 操作 |
|------|------|
| 4.4.1 | 在 `set_rect` 中引入脏检查：仅在 tabs 列表变化时重建 `Vec<ListItem>` |
| 4.4.2 | 评估 `title: Arc<str>` 替代 `String::clone()` |

### 4.5 消除 `tab_infos` 每帧双 clone（解决 B3）

`app_renderer.rs:328,342` 中 `tab_infos` 分别 clone 给 set_tabs_input 和 set_sidebar_input。改为通过引用传递或 share。

### Phase 4 验证

- `cargo test` 全通过
- 手动测试：hover 切换 tab、光标样式变化、sidebar 自动隐藏/显示、滚动动画
- 性能：`cargo build --release` benchmark 对比优化前后 set_rect 耗时

---

## Phase 5：渲染代码去重（预计 4 小时）

> **新增 Phase。** 原方案未覆盖 render_pipeline.rs 中的大量代码重复。

### 5.1 提取 whitespace cluster advance 辅助函数（解决 B4）

`render_pipeline.rs` 中 6 处重复的：

```rust
let is_ws = is_whitespace_cluster(&line_bytes[c.byte_range.clone()]);
let adv = if is_ws { ws_cluster_advance(&line_bytes[c.byte_range.clone()], char_width) } else { c.advance.max(1.0) };
```

提取为：

```rust
fn cluster_advance(cluster: &GlyphCluster, line_bytes: &[u8], char_width: f32) -> (bool, f32) {
    let is_ws = is_whitespace_cluster(&line_bytes[cluster.byte_range.clone()]);
    let adv = if is_ws { ws_cluster_advance(&line_bytes[cluster.byte_range.clone()], char_width) } else { cluster.advance.max(1.0) };
    (is_ws, adv)
}
```

替换 render_pipeline.rs 行 578-579, 588-589, 660-661, 734-736, 804-805, 846-847。`layout.rs:27-28` 中类似的模式也可一同替换。

### 5.2 消除 `visual_lines.clone()` 每行分配（解决 B5）

`render_pipeline.rs:576,586,879` 中 `visual_lines.clone()` 每可见行产生一次小 Vec 分配。评估是否可以：
- 缓存 visual_lines 结果
- 或用 `Rc<[(usize, usize, f32)]>` 共享

### 5.3 sidebar.rs 中 `28.0` vs `24.0` 不一致（解决 B6）

`sidebar.rs:456,979` 中 `let item_h = 28.0 * dpi` 与 `ROW_H = 24.0` 值不同（28 vs 24）。确认哪个是正确的 item 高度，使用对应常量。

---

## Phase 6：功能缺陷修复（预计 3 小时）

> **新增 Phase。** 原审计完全未发现的功能性问题。

### 6.1 用户设置持久化（解决 A1）

| 步骤 | 操作 |
|------|------|
| 6.1.1 | 在 `app.rs` 退出逻辑或设置变更时调用 `settings_io::save()` |
| 6.1.2 | 确认 `save()` 函数实现正确（已在 `settings_io.rs:32` 定义但从未调用） |

### 6.2 最近文件菜单（解决 A3）

| 步骤 | 操作 |
|------|------|
| 6.2.1 | 在文件打开逻辑中向 `RECENT_FILES` 写入条目 |
| 6.2.2 | 确认原生菜单构建代码能从 `RECENT_FILES` 读取 |

### 6.3 导航历史接入（解决 A2）

**现状：** `go_back()`/`go_forward()` 在 workspace.rs 定义但零调用。`app.rs:2077` 有 `// TODO: restore from tab_history` 注释。

| 步骤 | 操作 |
|------|------|
| 6.3.1 | 评估是否需要快捷键绑定（如 Cmd+Shift+[ 后退，Cmd+Shift+] 前进） |
| 6.3.2 | 或在确认不需要后删除死代码，移除 TODO |

### 6.4 mouse::handle_cursor_moved 死代码确认（解决 A4）

| 步骤 | 操作 |
|------|------|
| 6.4.1 | 确认鼠标拖拽选中是否通过 events.rs 的其他路径工作 |
| 6.4.2 | 若已有替代路径，删除 `mouse.rs:95`；若缺失，接入它 |

---

## Phase 7：依赖与 Crate 结构调整（预计 1 天）

与初版相同。

### 7.1 workspace.dependencies 统一

| # | 任务 |
|---|------|
| 7.1.1 | `cosmic-text` 移到 `[workspace.dependencies]` |
| 7.1.2 | `unicode_categories` 移到 workspace |
| 7.1.3 | `shaping` re-export app 需要的 cosmic-text 类型，让 app 不直接依赖 |

### 7.2 stdext 清理

| # | 任务 |
|---|------|
| 7.2.1 | 删除 `stdext::alloc` + `stdext::glob` |
| 7.2.2 | 评估 `stdext::simd`（仅 1 处引用） |

---

## Phase 8：文档归档与测试整理（预计 1.5 小时）

与初版相同，无修改。

---

## 修正汇总：相比初版的变化

| 初版内容 | 修正 | 原因 |
|---------|------|------|
| Phase 1 删除 `traffic_light_inset` | **移除** | 被使用 |
| Phase 1 删除 `rebuild_and_layout` | **移除** | 被使用 |
| Phase 1 删除 `ScrollbarAction` | **移除** | 被使用 |
| Phase 3.1 重命名 `ui::core` | **移除** | 不存在命名冲突 |
| Phase 4 重写事件分发为递归 | **改为** 消除 events.rs Workaround 代码 | Dock 已是递归分发 |
| Phase 4 删除 `steal_state`/`inject_state` | **移入** Phase 1 死代码清理 | 确认为死代码（仅测试用） |
| Phase 4 删除 update_layout 的 items 计算 | **移除** | update_layout 被渲染使用，不是冗余 |
| — | **新增** Phase 4 消除 TabBar 双重布局 | 每帧计算两次是真正的冗余 |
| — | **新增** Phase 5 渲染代码去重 | 6 处复制粘贴 |
| — | **新增** Phase 6 功能缺陷修复 | 设置丢失、最近文件为空、导航历史死代码 |
| — | **新增** `helpers::Size/Rect` 删除 | 全项目零引用 |
| — | **新增** `load_pinned` / `AppCommand` allow(dead_code) 移除 | 标注错误 |

## 附录 A：风险矩阵

| 阶段 | 风险 | 缓解 |
|------|------|------|
| Phase 0-1 | 低 — 删除死代码影响编译 | 每步 `cargo check` |
| Phase 2 | 低 — 常量值错误导致像素偏移 | 视觉回归 |
| Phase 3 | 高 — Sidebar 合并 | 先做小组件；独立 PR |
| Phase 4 | 高 — 涉及 events.rs 核心事件流 | 充分手动测试 hover/click/scroll |
| Phase 5 | 中 — visual_lines clone 影响性能语义 | benchmark 对比 |
| Phase 6 | 中 — 设置持久化需要选对触发时机 | 先讨论方案 |
| Phase 7-8 | 低 | — |

## 附录 B：建议执行顺序

```
Phase 0 → Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5 → Phase 6 → Phase 7 → Phase 8
```

- Phase 0-2 连续执行（约 6.5 小时），为后续重构清障
- Phase 3 和 Phase 4 有逻辑依赖：先合并双层架构，再优化事件流
- Phase 4（TabBar 双重布局）可与 Phase 3 并行（影响不同文件）
- Phase 6（功能缺陷）可提升到 Phase 1 之后独立执行

## 附录 C：Phase 3 原子化 PR

1. **PR1**: Scrollbar 合并（最小试探）
2. **PR2**: StatusBar + TitleBar 合并（小型组件）
3. **PR3**: PopupMenu 合并（中型组件）
4. **PR4**: Sidebar 合并（大型组件）
5. **PR5**: tab_bar re-export 整理
