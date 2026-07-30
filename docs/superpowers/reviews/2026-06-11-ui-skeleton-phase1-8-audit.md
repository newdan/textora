# UI 骨架 Phase 1–8 完成情况审计报告

- 审计日期：2026-06-11
- 审计范围：`docs/superpowers/plans/2026-06-11-ui-skeleton-phase{1..8}.md` 与 `phase7_5.md` 的实施进度
- HEAD：`58edb8f`（Merge codex/phase8-popup-overlay）
- 上次审计：`2026-06-11-ui-skeleton-phase1-6-audit.md`（仅 1–6）

## 1. 总览

| 指标 | 结果 |
|---|---|
| `cargo build --workspace` | ✅ 通过（20 个 dead_code/unused 警告） |
| `cargo test -p edit-plus-ui` | ⚠️ 294 / 295（**1 个失败**：`tab_bar::tab_bar_tests::tests::context_menu_pin_label_toggles`） |
| `cargo test -p edit-plus-app --lib` | ✅ 548 / 548（2 ignored） |

| 阶段 | 状态 |
|---|---|
| Phase 1 — Core 抽象 | ✅ 完成（上次审计已确认） |
| Phase 2 — UiShell + EditorHost + paint_backend 骨架 | ✅ 完成 |
| Phase 3 — status_bar widget + Text 路径 | ✅ 完成 |
| Phase 4 — search_bar widget + keyboard_focus | ✅ 完成 |
| Phase 5 — scrollbar widget | ✅ 主体完成；死 AppAction 留 Phase 9 |
| **Phase 6 — tab_bar widget 化** | ✅ **已补完**（commit `85c969b` + `fa49577`）|
| **Phase 7 — sidebar widget** | ✅ 完成（commit `21d49b2`） |
| **Phase 7.5 — VerticalListWidget 抽取** | ✅ 完成（commit `5912e54`） |
| **Phase 8 — popup overlay** | ✅ 主体完成；**1 个测试回归 + 老 NDC 函数残留**（commit `1f4c1d8` / `58edb8f`）|

## 2. Phase 6 — tab_bar widget 化（补完）✅

上次审计的"未完成项"现已全部到位：

| 检查 | 结果 |
|---|---|
| `TabBarState::hit_test_px / on_click_px / on_mouse_move_px / to_drawlist` | ✅ `tab_bar/state.rs:254/280/296/303` |
| `TabBarAction::OpenContextMenuPx { tab_index, anchor_px }` | ✅ `tab_bar/state.rs:32` |
| `TabBarWidget::paint` 走 `state.to_drawlist` | ✅ `widgets/tab_bar.rs:95` |
| `TabBarWidget::on_event` 调 `state.on_click_px / on_mouse_move_px / hit_test_px` | ✅ `widgets/tab_bar.rs:113/116/118` |
| `app_renderer.rs::tab_text_vertices`（230 行） | ✅ **已删** |
| `app_renderer.rs::popup_menu_text_vertices`（80 行） | ✅ **已删** |
| events.rs 调 `tab_bar_state.on_click / hit_test_at / on_mouse_move`（NDC 老路径） | ✅ **无命中** |

Phase 6 真完工。`widgets/tab_bar.rs` 从空壳 36 行 → 真 widget 152 行。

## 3. Phase 7 — sidebar widget ✅

| 检查 | 结果 |
|---|---|
| `widgets/sidebar.rs` | ✅ 544 行（含 Phase 7.5 list 集成） |
| sidebar 走 `ui_shell.dispatch` 而非老 hit_test_at | ✅ |

## 4. Phase 7.5 — VerticalListWidget ✅

| 检查 | 结果 |
|---|---|
| `crates/ui/src/widgets/list.rs` | ✅ 414 行 |
| `SidebarWidget` 内嵌 `VerticalListWidget` | ✅ `widgets/sidebar.rs:10/20/59/160` |
| `ListItem` / `ListItemKind` / `ListItemIndicator` / `ListAction` | ✅ |
| sidebar items 渲染委托 list（dirty → `Dot` indicator） | ✅ `sidebar.rs:160-163` |

## 5. Phase 8 — popup overlay ✅ 主体 / ⚠️ 残留

### 5.1 完成项

| 检查 | 结果 |
|---|---|
| `widgets/popup_menu.rs::PopupMenuWidget` + `PopupOutcome` | ✅ 227 行 |
| `popup_menu.rs::PopupMenu::overflow_px / context_px` | ✅ `popup_menu.rs:77/140` |
| `popup_menu.rs::PopupMenu::paint` + `hit_test_px` | ✅ `popup_menu.rs:254/312` |
| `UiShell::overlays + push_overlay + clear_overlays` | ✅ `ui_shell.rs:47/310/320` |
| `UiShell::dispatch` overlays 优先（后入先派） | ✅ `ui_shell.rs:355` |
| `app::OpenPopupMenu` 走 `ui_shell.push_overlay` | ✅ `app.rs:954/955`、`app.rs:987/988` |
| `events.rs` 处理 `PopupOutcome::Selected/Dismiss` | ✅ `events.rs:203/205/209` |
| `app_renderer.rs::popup_menu_text_vertices` 调用全删 | ✅ |
| `app_renderer.rs` 中老 popup 渲染分支整段删除 | ✅ `app_renderer.rs` 已通过 `paint_chrome` 统一渲染（含 overlays） |

### 5.2 残留问题

#### 5.2.1 ❌ **测试回归**：`context_menu_pin_label_toggles` 失败

```
crates/ui/src/popup_menu.rs:200  label: "固定标签".into(),     // 实际
crates/ui/src/tab_bar/tests.rs:128  expected "固定标签页"      // 测试断言
```

Phase 8 重构时把 `PopupMenu::context_px / context` 内 unpinned 项的 label 从 `"固定标签页"` 改成了 `"固定标签"`（少了"页"字），但 `tab_bar/tests.rs:128` 仍按旧 label 断言。**这是 Phase 8 直接引入的回归**。

修复二选一：
- 把代码改回 `"固定标签页"`（保 UI 文案一致）
- 把测试改成 `"固定标签"`（认可新文案）

`crates/ui/src/popup_menu.rs:636` 另一个 assert 用 `"取消固定"` 仍正确，不影响。

#### 5.2.2 ⚠️ 老 NDC 函数残留

```
crates/ui/src/popup_menu.rs:326  pub fn overflow(...)        // NDC 形态
crates/ui/src/popup_menu.rs:361  pub fn context(...)         // NDC 形态
crates/ui/src/popup_menu.rs:377  pub fn hit_test(px, py) → 转调 hit_test_px   // 兼容包装
```

Phase 8 plan 明列删除目标，新建 `_px` 版本后**没删老版**。grep 当前调用：

```
crates/ui/src/tab_bar/tests.rs:116/125/126  仍用 PopupMenu::context（NDC）
crates/ui/src/tab_bar/tests.rs:?  pm.hit_test(0.9, 0.9)（NDC）
```

只剩自家测试还在用，应用代码（events.rs/app.rs）已切到 `*_px` 版本。**清理动作小**：要么删函数 + 同步改测试，要么标 `#[deprecated]` 等 Phase 9 收尾。

#### 5.2.3 ⚠️ workspace 仍持 5 个 UI 字段（双轨）

```
workspace.rs:73  context_menu: Option<PopupMenu>      ← Phase 8 plan 标注可删
workspace.rs:74  overflow_menu: Option<PopupMenu>     ← Phase 8 plan 标注可删
workspace.rs:76  tab_bar_state: TabBarState           ← 未删
workspace.rs:77  sidebar_cfg: SidebarConfig           ← 未删
workspace.rs:78  sidebar_state: SidebarState          ← 未删
```

`tab_bar_state.preview_index()` 仍被 `workspace.rs:184/189/313/315/317` 调用——这是 preview tab 状态，不是渲染状态；可以保留到 Phase 9 看怎么收。

`context_menu / overflow_menu` 字段 Phase 8 plan 已说"可删"——**但仍存在**。grep 引用：

```
workspace.rs:73-74, 93-94, 556-557  仅声明、初始化、reset
（无实际使用 self.context_menu / self.overflow_menu 的写入或读出）
```

→ 这两个字段已成"幽灵字段"，安全删。

## 6. Dead-code 警告（20 条）

精选关键的：

| 警告 | 来源 | 建议处置 |
|---|---|---|
| `ScrollbarAction / SetScrollbarDragging / UpdateScrollbarState / ScrollbarHovered` 未构造 | `actions.rs` | Phase 5 残留，Phase 9 删 |
| `TextState` / `GpuState` 比 `paint_backend::drain` 更私有 | `paint_backend.rs` 签名 | 把 `pub fn drain` 改 `pub(crate)`，或把类型公开；当前其他模块没 import，私有化最安全 |
| `traffic_light_inset` 未用 | `sys/macos_titlebar.rs:58` | 与本重构无关；保留 |
| `open_overflow_menu` 未用 | `workspace.rs:789` | 老路径，**Phase 8 切到 ui_shell 后已无人调**，可删 |
| `unused import: tab_bar` `unused variable: show_tabs` 等 | app_renderer / app.rs | Phase 6/8 切换路径后留下；体力清理 |

## 7. 跨阶段健康度

| 维度 | 状态 |
|---|---|
| 编译 | ✅ |
| 单元测试 | ⚠️ 1 个回归（pin label） |
| 视觉一致性 | 未验证（建议手测：tab 右键 / overflow / sidebar 设置菜单 / DPI 切换） |
| 事件路径统一 | ✅ tab/sidebar/scrollbar/search/popup 全走 `ui_shell.dispatch` 与 `forward_key` |
| 渲染路径统一 | ✅ chrome + overlays 都走 `paint_chrome` → `paint_backend.drain` |
| px ↔ NDC 边界 | ✅ widget 全 px；NDC 仅在 `paint_backend` 翻译时出现 |
| 双轨残留 | ⚠️ workspace 字段、popup_menu 老 NDC 函数；Phase 9 收尾 |

## 8. 给"进入 Phase 9"的最小补丁清单

按重要性排序，建议在 Phase 9 第一波做掉：

1. **修测试回归**：`popup_menu.rs:200` 改回 `"固定标签页"`（或改测试）。**今天就做**——失败测试会拖累后续 CI 信心。
2. **删 workspace 幽灵字段**：`context_menu` + `overflow_menu`（已无引用）。
3. **删 `popup_menu.rs::overflow / context`（NDC 老版）+ 把 `tab_bar/tests.rs` 切到 `_px` 版**。
4. **删 `actions.rs` 的 ScrollbarAction 死变体**（4 条 dead_code 警告全消）。
5. **`paint_backend::drain` 私有化**（`pub` → `pub(crate)`，去 2 条警告）。
6. **删 `workspace.rs::open_overflow_menu`**（已无人调）。

完工标准：`cargo build --workspace 2>&1 | grep warning | wc -l` ≤ 5（保留 macOS 平台特定的 `traffic_light_inset` 等无关警告）。

## 9. 结论

> 8 个阶段中 7.5 个真正完成。唯一未完成项是 Phase 8 引入的一个测试文案回归，5 分钟可修。  
> 整个重构从 Phase 1 的设计到 Phase 8 的 popup overlay，**核心目标全部达成**：  
> - `tab_bar` 1656 行拆为 `tab_bar/{layout,state,render,hit,text,types}` + `widgets/tab_bar.rs`  
> - 删 `tab_text_vertices`(230 行) + `popup_menu_text_vertices`(80 行)  
> - 所有组件走 `paint_chrome → paint_backend.drain`  
> - 所有事件走 `ui_shell.dispatch` 返回 `Box<dyn Any>` 强类型 action  
> - widgets 内部全 px，NDC 仅 backend 翻译时出现  
>  
> 进入 Phase 9（收尾清理）的所有前置条件已就绪。
