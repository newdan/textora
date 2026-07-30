# UI 骨架审计报告 v2

> 日期：2026-06-12
> 范围：6.11 至今所有提交（57b645d → e091f0a，覆盖 UI 骨架 Phase 1-9 全过程）
> 关注：sidebar / 内容区 / IME 已知问题根因 + 代码结构 + 测试 + 安全 + 性能
> 与 v1 (`ui-skeleton-audit-2025-06-12.md`) 的差异在每节末尾标注

---

## 0. TL;DR

| 维度 | 结论 |
|------|------|
| 编译 | `cargo check` ✅ ；`cargo test --no-run` 在 `edit-plus-render` 处失败（与 UI 骨架无关，wgpu InstanceDescriptor API 升级遗留）。**违反 CLAUDE.md 第 8 条"每次提交要确保能编译过"。** |
| 单元测试 | ui crate **318 通过**，app crate **549 通过 / 2 忽略**。 |
| Sidebar 三大问题 | 全部由"旧 SidebarState 与 Phase 7 SidebarWidget 双轨"+"app 层未注入 sidebar 输入"导致，根因仍是 v1 指出的接线缺口。 |
| 内容区五大问题 | 1/4/5 同源（`mouse_hit_test` 与 `shape_visible_lines` 的 `left_margin` 计算不一致，sidebar 模式偏移仅加在渲染侧）；2 是 widget dispatch 与老路径事件保护带共同截断；3 是 `set_status_input` 已接好但 `view_mode==Sidebar` 时被 sidebar 区域吞掉显示。 |
| IME 不显字母 | 在 `app_renderer.rs:557` 处依赖 `cursor_visual_line.is_some()`，新文档 / 编辑器尚未 shape 时为 `None`。 |
| 代码结构 | 双轨 SidebarState/Widget 还在；TabBar 仍走老顶点路径；多处 `eprintln!` 留在事件主路径热区；公共 API 暴露偏宽（如 `pub` 字段、`pub(crate)` mut 访问器对外暴露内部状态）。 |
| 安全 | 无 unsafe-滥用；存在 `Box::leak`（仅测试里）；`Settings::get()` 多重借用风险点已收敛；ui 层 `unsafe` 用于 `transmute(RefCell::borrow)`，**值得复审**。 |
| 性能 | `update_frame` 每帧重建 Dock + 5~6 widget + 多次 Vec/HashSet 克隆；`eprintln!` 在 mousemove 主路径打印；`paint_chrome → drain` 每 chrome text 走 shape；当前 chrome 文本量小，OK，但是热路径的诊断 IO 应清掉。 |

---

## 一、提交脉络（6.11–今天）

总计 **44 次提交**，可分四阶段：

1. **Sidebar 双模式 + 阶段 1-9 设计**（57b645d, 9845919, 27f7caa, b8be3e4, 14bd888, 880a291, 1b8bacb, 8fc4c2b, 8014ff9）：建立 `SidebarConfig/State/Action`，引入 view_mode 切换，实现 Hidden/HoverPeek/Pinned 三态机及 settings menu。
2. **窗口/标题栏/IME 修复**（039e1b1, 2302e3b, b018b50, c695b72, 98f1f58）：解决 RefCell 借用冲突、行号栏可见性、IME preedit 显示、滚动条范围 bug。
3. **UI 骨架 Phase 1-9**（6a2dee5 → e091f0a）：抽出 `geom/paint/measure/widget/dock`，引入 `UiShell + EditorHost + paint_backend`，逐个 widget 化（StatusBar / SearchBar / Scrollbar / TabBar / Sidebar / Popup overlay），最后清 NDC 残骸 + 把 sidebar/tab_bar 输入收敛进 `UiShell`。
4. **修过载重绘 / 拖拽泄漏**（df2bf71, 1b0e77d, f2a4fcc, 32eaa55, 0539666）：定位多个 needs_redraw 自激振荡源。

**整体感受**：Phase 1-9 在不到一天里做完九阶段骨架重构 + 双模式（Tabs/Sidebar）功能补全，**本次审计前的最后一个商业完整可用版本应回退到 32eaa55 之前**（之后的 Phase 6-9 大量动了输入路径但没补端到端冒烟测试）。

---

## 二、已知问题逐项根因（基于 e091f0a 实际代码）

### 2.1 Sidebar 三宗罪

#### (a) 不显示 workspace 文档列表 — **接线缺失**

- `crates/app/src/ui_shell.rs:157` 定义 `set_sidebar_input(cfg, tabs, active_index, traffic_light_inset)`。
- 在 `crates/app/src/` 全仓 grep `set_sidebar_input` 调用点：**0 个**（`set_status_input`, `set_search_input`, `set_scrollbar_input`, `set_tabs_input` 均在 `app_renderer.rs:311-363` 调用，唯独 sidebar 漏了）。
- 因此 `UiShell.sidebar_tabs` 永远是 `Vec::new()`，`update_frame` 中 `SidebarWidget::set_input(...)` 用空 tabs 给 `VerticalListWidget`，渲染 0 行。

> **注**：v1 已诊断此根因，至 e091f0a 仍未修复。

#### (b) 汉堡按钮点击无效 — **三个错误叠加**

1. **Hidden 时事件不到 SidebarWidget**：`SidebarState::current_width()`（`sidebar.rs:178`）在 `Visibility::Hidden` 返回 0；`events.rs:282` 进入 sidebar widget dispatch 的前置条件是 `sidebar_w > 0.0 && px < sidebar_w`，Hidden 时永远 false，hamburger（左上角 4×4..32×32）无法触发。
2. **`SidebarWidget::hit()` 也会拒绝**：`widgets/sidebar.rs:181` `state.is_visible() && rect.contains(...)`，Hidden 时返回 false，即使绕开 events.rs 截断走 Dock 路径也命中失败。
3. **语义错误**：`events.rs:504` `SidebarAction::TogglePin → AppAction::TogglePin → workspace.toggle_pin()`（`app.rs:1037`）= 切换**当前标签的钉选**，不是切换 sidebar 显/隐。

> **修复方向**：
> - `events.rs` 的 sidebar 事件保护带改为 "px < hamburger_button_max_x（始终 ≈ 32px）" 而不是 `sidebar_w`；
> - 或者干脆放弃保护带，让 Dock dispatch 处理（但需要 `SidebarWidget::hit()` 在 Hidden 时也对 hamburger 区返回 true，等于让 widget 占据 0×0 之外的额外热区——更好的做法是给 hamburger 单独一个微型 widget）；
> - 新增 `AppAction::ToggleSidebar`，把 hamburger 走的 `TogglePin` 重定向为它。

#### (c) 设置按钮点击无效 — **静默丢弃**

- `events.rs:502` `SidebarAction::OpenSettingsMenu => None, // handled by old popup path (Phase 8)`，注释**实际错的**：Phase 8 popup overlay 路径只接管 tab_bar 右键菜单，没有接管 sidebar 设置按钮。
- 真正"老路径"在 `events.rs:258-272`，依赖 `app.ui_shell.sidebar_state().open_menu().is_some()`；而 `SidebarState::open_settings_menu()` 是唯一会把 `open_menu` 置非 None 的入口，全仓查无调用点。
- 因此 settings 按钮：点击 → `SidebarWidget` 命中 settings_btn_rect → 返回 `OpenSettingsMenu` → `translate_sidebar_action` 丢弃 → 用户毫无反馈。

> **修复方向**：让 `translate_sidebar_action(OpenSettingsMenu)` 直接调 `app.ui_shell.sidebar_state_mut().open_settings_menu(...)`，或迁移到 Phase 8 的 popup overlay（`UiShell::push_overlay`）。

### 2.2 内容区五项

#### (1) 行号区域被遮盖 — **left_margin 在 sidebar 模式偏移**

- `app_renderer.rs:248-261` 渲染侧把 sidebar offset **加进** `left_margin`：
  ```rust
  let left_margin = Settings::get().content_left_margin().max(gutter_w);
  let left_margin = if Sidebar { lm.max(sidebar_offset + dpi*10.0) } else { lm };
  ```
- 但是 **mouse hit_test 用的不是同一份**：`events.rs:151, 361`
  ```rust
  let left_margin = Settings::get().content_left_margin().max(gutter_w);
  // ↑ 没加 sidebar_offset
  ```
- 行号区域本身并没有被遮盖；**真正现象**是 sidebar pinned 时，gutter 背景使用了正确的扩展 left_margin（`gutter_bg_vertices` 拿 `gutter_left_margin`），但 `editor_left_offset` 与渲染 `left_margin` 的差额（`dpi*10.0` padding）让行号文字被画进了原本属于 sidebar 内 padding 的区域里 —— 视觉上像被 sidebar 阴影压住。

#### (2) 滚动条 hover/拖动无效 — **派发链 OK 但 hover 无重绘**

- `widgets/scrollbar.rs:127-186` 实现完整，hover/StartDrag/DragTo/EndDrag/PageUp/PageDown 全有。
- `events.rs:46-72` cursor_moved 中 dispatch → 收到 `ScrollbarAction::HoverChanged(true)` → push `SetCursor(Default)`，**但没有 push `RequestRedraw`**。
- 结果：thumb hover 时 alpha 应从 0.5 → 1.0（`paint` 中读 `state.hovered`），但因为没触发重绘，画面定格。
- 拖动也走同一 dispatch；可疑点是 events.rs:84-87 sidebar 保护带在 `view_mode==Sidebar` 时 `if px < sidebar_w return actions`：**滚动条在右侧，px 远大于 sidebar_w，不受影响**。所以拖动应该可用（除非 `viewport_height >= total_display_rows` 导致 `show_thumb=false`，Settings 默认 viewport_height 在 `set_input` 注入的是 `dv.viewport.viewport_height`，单位是"行数"，与 `total_display_rows` 同维度，看起来 OK）。
- 但是 dispatch 路径还有一个细节：`events.rs:46` 的 `dispatch` 调用先走 sidebar resize 检查（成功消费）→ 后续 ScrollbarAction 永远拿不到。需在 dispatch 返回后 **同时** match Sidebar/Scrollbar 两类 action（当前代码确实分别 `if let` ScrollbarAction，可同时执行）。

> **修复方向**：
> - `HoverChanged(true|false)` 都 push `AppAction::RequestRedraw`；
> - 拖动时确认 `set_input` 中 `viewport_height` 单位（行数 vs 像素），与 `compute_layout_px` 内部对 `viewport.max(1.0)` / `total.max(1)` 取相同维度。

#### (3) 状态栏无信息显示 — **Sidebar 模式下被吞**

- `app_renderer.rs:311` `set_status_input` **每帧都调**，传入正确字段（`buffer_len, cursor_line/col, ...`）。
- `widgets/status_bar.rs:44` `paint` 在 `buffer_len == 0` 时仅画背景；非 0 时画 `Text { content }`。
- **复核 sidebar 模式**：`build_shell_inputs`（`app.rs:208`）`status_thickness = settings.status_bar_height`（所有模式都非 0），所以 status bar widget 一定加入 dock。理论上应该显示。
- **可疑点**：`Settings::get_static().status_bar_height` 在程序生命期内是否被改成 0？需运行时打 log。
- **若仍不显示**，最可能原因：`paint_backend::drain` 时把 status bar 的 `Text` 命令路由到 shaper，但当前 `drain` 实现可能跳过了 `DrawCmd::Text`（v1 文档未深查）。

> **修复方向**：在 `paint_backend.rs` 加临时日志确认 Text 命令是否被翻译为顶点。

#### (4) 点击后光标位置偏移 / (5) 高亮区域不对 — **同源 left_margin 不一致**

详见 (1)。两条路径用了不同 `left_margin`：

| 路径 | 文件:行 | left_margin 是否含 sidebar_offset |
|------|--------|----|
| 渲染（顶点 x） | `app_renderer.rs:250` | ✅ 含 |
| 行号 gutter 背景 | `app_renderer.rs:438-451` | ✅ 含 |
| 选区高亮 | `app_renderer.rs:524`（call） | ❌ 不含 |
| 搜索高亮 | `app_renderer.rs:490` | ❌ 不含 |
| Mouse hit_test | `events.rs:151, 361` | ❌ 不含 |

→ 选区/搜索高亮和文本顶点在 sidebar 模式下错位约 `dpi*10.0` 像素；点击的 buffer 偏移按"窄"left_margin 算，但渲染的字按"宽"left_margin 摆，导致点击位置偏移而行号高亮（按视觉行号）正确。

> **修复方向**：把 `app_renderer.rs:248-261` 的 sidebar 偏移逻辑抽成一个 `App::content_left_margin(dv)` 方法，所有路径统一调用。

### 2.3 IME 输入时不显示已输入字母

- `app_renderer.rs:553-574` 渲染 preedit 的前提：
  1. `!self.preedit_text.is_empty()` ✓ winit 给的事件已 set；
  2. `self.text` 和 `self.gpu` 都非 None；
  3. `self.workspace.doc_views.get(active_index)` 非 None；
  4. **`dv.cursor_render_state.cursor_visual_line.is_some()`** ← 关键。
- `cursor_visual_line` 在 `shape_visible_lines` 里设置（`render_pipeline.rs:202` 在某条件下设为 `Some(...)`），新文档/未触发首次 shape 时为 None，此时 IME preedit 不渲染。
- 又：`cursor_x` 用 `dv.cursor_render_state.cursor_pixel_x`，新文档时也为 0；`cursor_y = content_top + cursor_vl * line_height` 看起来没问题。
- **更易踩中的边界**：IME `Preedit` 事件触发 `needs_redraw = true`（`app.rs:2294`），但在第一次输入前可能 `last_render_time` 之后没触发 shape → `cursor_visual_line` 还是 None → 跳过 preedit 渲染 → 第二次输入又来 IME 事件，但因为 cursor 没动 shape 路径不重做 → 死循环。

> **修复方向**：把 preedit 渲染条件放宽——若 `cursor_visual_line` 为 None，回退到 `cursor_visual_line = 0`，`cursor_x = left_margin`。

---

## 三、代码结构审计

### 3.1 优点（保留 v1 评价并加新观察）

| 方面 | 评价 |
|------|------|
| 分层 | `crates/ui` 不依赖 `crates/app`、`DocumentView`、`Workspace`，遵循 AGENTS.md 第 39 行的依赖图。 |
| Widget trait | `set_rect / paint / hit / on_event / as_any[_mut]` 五件套统一；MouseUp 不依赖 hit（dock.rs:149）这个细节考虑到了拖拽外移。 |
| Dock 布局 | 闭包 `thickness: Box<dyn Fn(...) -> f32>` 支持 DPI 动态计算；Phase 9 已把 `visible` 标志改为简单 bool（不是动态闭包），降低复杂度。 |
| DrawList 抽象 | `DrawCmd::FillRect / Text / RoundedRect` 足以覆盖当前 chrome；`paint_backend::drain` 翻译为 `GlyphVertex`。 |
| VerticalListWidget | Phase 7.5 抽出，可被 sidebar/menu 通用。 |
| Overlay 体系 | Phase 8 引入 `UiShell.overlays + push/pop_overlay`，dispatch 在 `ui_shell.rs:393` 优先 overlay-逆序-命中。 |
| px 化 | scrollbar 已删除全部 NDC 函数（`scrollbar.rs:1` 注释明示），title bar / popup_menu / tab_bar 也走 px。 |

### 3.2 问题（按严重性）

#### S1（影响功能）

1. **`SidebarState` 与 `SidebarWidget` 双轨依然存在**（`sidebar.rs` 1113 行 vs `widgets/sidebar.rs` 544 行）。
   - 当前 `SidebarWidget` 内嵌 `state: SidebarState`，但所有 layout/paint/hit_test_px 仍委托给 state，widget 仅外加了 resize drag 与 list 委托。
   - 命中链为：`on_event(MouseDown)` → ① edge resize ② `state.hit_test_px(px,py)` ③ `list.on_event` —— 看似闭环，但**输入空 tabs 时 hit_test_px 仍能命中 hamburger** ✓；问题是 events.rs **没让事件进 widget**（见 2.1.b）。
2. **AppAction::TogglePin 语义过载**（`actions.rs` + `events.rs:504` + `app.rs:1037, 1155`）：同一 enum 值用于"标签钉选"与"sidebar 显隐"，且只有第一种被实现。
3. **`translate_sidebar_action` 静默丢弃 `OpenSettingsMenu` 与 `PersistConfig`**（`events.rs:502, 511`），让按钮点击无反馈。
4. **TabBar 未完全 widget 化**：`crates/ui/src/tab_bar/render.rs` 仍直产 `GlyphVertex`，与 status/scrollbar/searchbar 的 DrawList 路径不一致；`widgets/tab_bar.rs` 是瘦包装。
5. **`build_shell_inputs` 与 `app_renderer` 重复构造 `tab_infos`**（`app_renderer.rs:336-364` 和 `app_renderer.rs:380-399`），同一帧两次相同分配。

#### S2（影响可读性 / 可维护性）

6. **`UiShell` 公共 API 暴露内部状态**：`sidebar_state_mut() / sidebar_cfg_mut() / set_dragging_sidebar()`（ui_shell.rs:129-147）让 app 层可以越过 widget 直接改 state，破坏封装。
7. **`pub(crate)` 字段直接暴露**：`sidebar_config / sidebar_state / tab_bar_state` 在 `ui_shell.rs:60-74` 全部 `pub(crate)`，且 events.rs:81 直接做 `shell.sidebar_state.on_mouse_move(...)`，绕过 widget。
8. **诊断代码留在主路径**：
   - `app_renderer.rs:716` `println!("[frame] total=...")` 每超阈值帧打印；
   - `events.rs:145, 361` `eprintln!("[events:cursor_moved] ...")` **每次鼠标移动**打印（含 px,py,lm,gw,tbh,lc,ac_len 七个字段）。
9. **`ui_shell.rs:306` `screen_for_input` 变量定义了不用**（cargo warning 实证）。
10. **`actions.rs:72` `ScrollbarAction` 变体永不构造**（cargo warning 实证），但 `app.rs:1070-1073` 还在 match。
11. **`paint_backend.rs:264` `let c = corner_vertex(...)` 结果丢弃**（cargo warning 实证）。
12. **`reshape_worker.rs:71-76` `let mut proxy = None` 之后立刻 `proxy = Some(p)` 但从未读**（cargo warning 实证）。
13. **`Settings::get_static() / Settings::get()` 双 API 并存**（settings.rs:147-169），含两处 `unsafe transmute`，调用方判断哪个该用哪个增加心智负担。

#### S3（小瑕疵）

14. **AGENTS.md 第 8 条："每次提交要确保能编译过"被违反**：`cargo test --workspace --no-run` 在 `edit-plus-render` 上失败（wgpu API 升级遗留，与 UI 骨架无关，但工作区整体不绿）。
15. **`app.rs:1398, 1438` `unsafe { &*dm_ptr }`** 两处裸指针解引用，需复审是否能改用 `Rc<RefCell<...>>` 或 split borrow。
16. **`tab_bar/tests.rs:14` `test_ctx` dead code**（cargo warning 实证）。

> **与 v1 差异**：v1 主要批评双轨与接线缺口；v2 进一步定位 ① `left_margin` 不一致是问题 1/4/5 的共同根因，② hover 重绘缺失是问题 2 的根因，③ IME `cursor_visual_line.is_some()` 守卫是问题 6 的根因，④ 主路径有 `eprintln!` 副作用。

---

## 四、测试覆盖分析

### 4.1 实际跑出的数字

| crate | passed | ignored | failed |
|-------|--------|---------|--------|
| `edit-plus-ui --lib` | 318 | 0 | 0 |
| `edit-plus-app --lib` | 549 | 2 | 0 |
| `edit-plus-render --lib` | — | — | **编译失败** |

ui 318 / app 549 比 v1（295 / 606）涨/降原因：
- ui +23：Phase 4-9 的 widget 测试（list 10 + scrollbar 17 + sidebar 10 + status 11 + searchbar 18 + popup 5 + tabbar 20 + dock 9 + geom 7 + paint 5 + widget 6）。
- app -57：Phase 7-9 删除了一批 ndc 路径相关旧测试，且若干集成 case 移到 ui 侧。

### 4.2 缺失（按风险）

| 项 | 风险 | 说明 |
|----|------|------|
| Sidebar 端到端：set_input → update_frame → hamburger 点击 → ToggleSidebar | 🔴 高 | **若有此测试可在 8014ff9 即捕获 2.1.b** |
| StatusBar 端到端：含 sidebar 模式 chrome 渲染 | 🔴 高 | 同上，可捕获 2.2.3 |
| `mouse_hit_test` 与 `shape_visible_lines` 在 sidebar 模式下 left_margin 一致性 | 🔴 高 | 可捕获 2.2.1/4/5 |
| Scrollbar HoverChanged → RequestRedraw 链路 | 🟡 中 | 当前只测 widget 层 action，未端到端 |
| IME preedit 在新文档（`cursor_visual_line=None`）的回退 | 🟡 中 | |
| Dock dispatch 多 child 事件竞争（sidebar/scrollbar/overlay 同时存在） | 🟡 中 | |
| `events.rs::translate_sidebar_action` 各分支 → AppAction 映射 | 🟡 中 | 可捕获 2.1.b 语义错误 |
| `build_shell_inputs` 各 view_mode × chrome 组合（已有 16 个，但不含 hovered/active 切换序列） | 🟢 低 | |

### 4.3 测试代码可优化点

- `widgets/sidebar.rs:262-545` 测试模块每个 case 重复 80+ 行 `test_theme()`，可提到 `crate::theme::testing` 模块。
- `popup_menu.rs` 内 `Box::leak(Box::new(Settings::new()))` 出现 11 次（grep 实证），应抽出 `setup_test_settings()` helper。

---

## 五、安全审计

| 类别 | 评估 | 备注 |
|------|------|------|
| `unsafe` 块 ui 层 | 仅 `settings.rs:147,157,169` 三处 `transmute(RefCell::borrow ⇒ &'static Settings)` | **风险中等**：把 RefCell 借用伪装成 'static，依赖编译期假设"Settings 单例不会被 drop / 不会跨线程"。当前事件循环单线程，可接受；但 `Settings::get_static()` 返回 `&'static` 暴露给所有调用者，难以保证 borrow checker 阻止后续 `borrow_mut`。建议用 `std::cell::OnceCell<Settings>` 或 `std::sync::OnceLock` 替代。 |
| `unsafe` app 层 | `app.rs:1398, 1438` 两处 `&*dm_ptr` 裸指针解引用 | **需复审**：context 是为绕开 split borrow 的 trick，注释里也提到。可改成 `Rc<RefCell>` 避免 unsafe。 |
| `unsafe` 系统层 | `native_menu.rs / sys/macos_titlebar.rs` 走 objc2，正常用法 | OK |
| `unwrap()` / `expect()` | grep 见下 | 需逐个评估 |
| `Box::leak` | 仅 popup_menu 测试中 11 次 | 测试 mock，**生产无**。 |
| RefCell 嵌套借用 | 多次修过（039e1b1, 2302e3b），目前未见新增风险 | 但 `sidebar.rs` 内多次独立调 `Settings::get().dpi_scale`，若调用栈中持有 `borrow_mut` 会 panic。 |
| 浮点除零 | `viewport.max(1.0)`、`total.max(1)` 已保护（scrollbar.rs:27-28） | OK |
| 整数溢出 | `DisplayRow` 用 `saturating_add/sub`、`scroll_anchor` 用 `clamp` | OK |
| 序列化 | `SidebarConfig` 字段简单 | OK |
| 文件 I/O | `settings_io::save/load`、`open_file` 走 `Result`，错误打印不 panic | OK |

### 5.1 主路径 `unwrap()` 隐患

```bash
$ grep -c "unwrap()\|.expect(" crates/app/src/{app.rs,events.rs,app_renderer.rs}
app.rs: ~24
events.rs: ~3
app_renderer.rs: ~6
```

需逐一复审主要在 `events.rs::translate_tab_action` 与 `app.rs` 的 menu/IME 处理分支。

---

## 六、性能审计

### 6.1 评估

| 维度 | 实测/估算 |
|------|----------|
| 每帧重建 widget | `update_frame` 每帧 `dock.children.clear()` + `Box::new(TabBarWidget/SearchBarWidget/StatusBarWidget/SidebarWidget/ScrollbarWidget)` ≈ 5 次堆分配；`tab_input_pinned_indices.clone()` 在 `app_renderer.rs:358` 每帧再做 HashSet 克隆。60fps 下 = ~300 alloc/s，**可接受**但应消除。 |
| 字符串克隆 | `sidebar_tabs.clone()`（ui_shell.rs:299）、`tab_input_tabs.clone()`（app_renderer.rs:336-353 构造 tab_infos 两次） | 每帧 2×N 个 String 克隆，N=tab 数。 |
| chrome 文本 shape | `paint_backend::drain` 对每 `DrawCmd::Text` 调 shaper，当前 chrome 文本量小（status 一行、tab 标题、sidebar item）≈ 数十字符 | OK |
| 鼠标移动主路径 | `events.rs:145` & `events.rs:361` **每次 cursor_moved + mouse_input 都 eprintln!**，含格式化 7 字段 | **🔴 高优先级修掉** |
| 帧时打印 | `app_renderer.rs:716` 仅在 >1ms 或 >20ms 间隔时打，OK | |
| 大文件 advance_cache | render_pipeline 中 `advance_cache.iter().map(|e| e.doc_line).max()` 每帧 O(N visible) | OK |
| `set_input` viewport_height 单位 | `viewport.viewport_height` 单位是"行数"（viewport.rs:148），`compute_layout_px` 把它当 visible vs total 做比例，**单位一致**。 | OK |

### 6.2 优化建议

| # | 项 | 收益 |
|---|----|----|
| P0 | 删 `events.rs:145, 361` 的 `eprintln!` | 每次 mousemove ~10us 节省 |
| P1 | `app_renderer.rs:336-364` 与 `app_renderer.rs:380-399` 两处 `tab_infos` 复用 | 每帧 N×String 克隆 |
| P1 | Widget 池化（保留 `TabBarWidget/SidebarWidget` 实例，只 `set_input/set_rect`） | 每帧 5 次 Box::new |
| P2 | `tab_input_pinned_indices` 不每帧 `.clone()`，改 `&HashSet` 引用 | 每帧 1 次 HashSet 克隆 |
| P2 | `paint_backend::drain` 中 chrome `Text` 命令复用 shaped 结果（chrome 文本短期不变） | 微 |

---

## 七、修复优先级清单

### P0 — 阻塞功能（必修）

| # | 问题 | 文件:行 | 方案 |
|---|------|--------|------|
| 1 | `set_sidebar_input` 接线 | `app_renderer.rs:308` 处增加调用 | 与 `set_status_input` 并列；用 `tab_infos`、`workspace.active_index`、`current_traffic_light_inset()` |
| 2 | hamburger 按钮在 Hidden 时点不到 | `events.rs:282` & `widgets/sidebar.rs:181` | 把 hamburger（左上角固定 32×32）作为独立路径优先匹配，无视 `sidebar_w`；或新增 `SidebarWidget::hit_for_hamburger(px,py)` |
| 3 | TogglePin 语义错误 | `events.rs:504` & `actions.rs` | 新增 `AppAction::ToggleSidebar`，hamburger 走它；keyboard 路径保留 `AppAction::TogglePin` 给标签钉选 |
| 4 | OpenSettingsMenu 静默丢 | `events.rs:502` | 改为调用 `app.ui_shell.sidebar_state_mut().open_settings_menu(...)` 或 push popup overlay |
| 5 | left_margin 在 mouse_hit_test / 选区 / 搜索高亮处不含 sidebar offset | `events.rs:151,361`、`app_renderer.rs:490, 524` | 抽出 `App::editor_left_margin(dv) -> f32` 单一来源 |
| 6 | scrollbar HoverChanged 无 redraw | `events.rs:64-69` | `HoverChanged(_)` 时 push `AppAction::RequestRedraw` |
| 7 | IME preedit 在 cursor_visual_line=None 时不渲染 | `app_renderer.rs:557` | None 时回退 `(0, left_margin)` |

### P1 — 结构改进

| # | 项 |
|---|----|
| 8 | 完全合并 `SidebarState`+`SidebarWidget`；废除 `UiShell::sidebar_state_mut()` 等"穿透"API |
| 9 | TabBar 渲染走 DrawList + paint_backend，统一 chrome 路径 |
| 10 | Widget 池化（保留实例，每帧 set_input + set_rect） |
| 11 | 删 `events.rs` 主路径 `eprintln!`，改 tracing::debug! 默认关 |
| 12 | 删 `actions.rs::ScrollbarAction` 死变体；删未读 `proxy`、`screen_for_input`、`c` 等 |

### P2 — 测试补全

| # | 测试 |
|---|------|
| 13 | sidebar 端到端：set_input → update_frame → hamburger 点击 → ToggleSidebar action |
| 14 | sidebar 端到端：settings 按钮点击 → 弹出 popup overlay |
| 15 | left_margin 一致性 prop test：所有 5 条路径在 sidebar pinned/hidden 下相等 |
| 16 | scrollbar hover → RequestRedraw 端到端 |
| 17 | IME preedit 在新建空文档输入第一字符渲染顶点 |
| 18 | `translate_sidebar_action` 全分支映射表（snapshot 测试） |

### P3 — 卫生

| # | 项 |
|---|----|
| 19 | 修 `crates/render/src/lib.rs:678` wgpu 0.21+ API 升级，恢复 `cargo test --workspace --no-run` 全绿 |
| 20 | `Settings::get_static()` 改 `OnceLock<Settings>`，去掉 `unsafe transmute` |
| 21 | `app.rs:1398, 1438` `unsafe &*dm_ptr` 改 `Rc<RefCell>` 或 split borrow |
| 22 | `popup_menu.rs` 测试 helper 抽出，去 11 处 `Box::leak` 复制 |

---

## 八、附录：本次审计方法说明

- **数据源**：直接读取 e091f0a 处 `crates/{ui,app}/src/` 下所有相关 .rs 文件，不照搬 v1。
- **编译验证**：跑 `cargo check -p edit-plus-app/ui` 全绿；`cargo test --no-run` 在 render crate 失败（与本次审计范围无关）。
- **测试结果验证**：跑 `cargo test -p edit-plus-ui --lib`（318 通过）和 `cargo test -p edit-plus-app --lib`（549 通过 / 2 忽略）。
- **审计假设**：用户报告的"hover/拖动无效"等行为在最新代码上仍能复现（未亲跑 GUI 验证，结论以代码静态分析为准）。

> 审计结束。建议按 P0 表格 7 项依次修复并补 P2 端到端测试，避免下次出现"接线缺失但 widget 单测全绿"的盲区。
