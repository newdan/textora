# UI 骨架 Phase 1–6 完成情况审计报告

- 审计日期：2026-06-11
- 审计范围：`docs/superpowers/plans/2026-06-11-ui-skeleton-phase{1..6}.md` 对应实施进度
- 审计依据：现仓库 `crates/ui` 与 `crates/app` 文件结构 + `cargo build/test` 实测
- HEAD 提交：`3dd51f3`（Merge branch 'codex/fix-excessive-redraw'）

## 1. 总览

| 指标 | 结果 |
|---|---|
| `cargo build --workspace` | ✅ 通过（10 个 dead_code/unused 警告） |
| `cargo test -p edit-plus-ui` | ✅ 275 / 275 |
| `cargo test -p edit-plus-app --lib` | ✅ 548 / 548（2 个 ignored） |
| `cargo test --workspace` | ⚠️ render crate lib-test 与 wgpu git rev 编译错误，整 workspace 测试目标无法编译；非本重构引入，不影响主程序 build 与运行 |

| 阶段 | 状态 | 备注 |
|---|---|---|
| Phase 1 | ✅ 完成 | core 五件套 + measure_adapter 全到位 |
| Phase 2 | ✅ 完成 | UiShell / paint_backend / EditorHostWidget 接入；render() 调 update_frame |
| Phase 3 | ✅ 完成 | StatusBarWidget；paint_backend Text 路径就绪；老 status vertices 函数已删 |
| Phase 4 | ✅ 完成 | SearchBarWidget；keyboard_focus + forward_key；ui::search_bar 老函数全删 |
| Phase 5 | ✅ 主体完成 | scrollbar 老 NDC API 全清；scrollbar.rs 从 660 行减到 107 行；**残留死 AppAction 等待 Phase 9 收尾** |
| Phase 6 | ❌ **仅完成文件拆分** | widget 化、px 化、老 vertices 删除——**全部未做** |

## 2. Phase 1 — Core 抽象就位 ✅

应交付（来自 plan §"文件结构"）：

| 项 | 实际 |
|---|---|
| `crates/ui/src/core/geom.rs` | ✅ 147 行 |
| `crates/ui/src/core/paint.rs` | ✅ 181 行 |
| `crates/ui/src/core/measure.rs` | ✅ 36 行 |
| `crates/ui/src/core/widget.rs` | ✅ 254 行；`as_any` / `as_any_mut` 默认实现 panic（要求子类显式覆写——**比 plan 多了这层防御**） |
| `crates/ui/src/core/dock.rs` | ✅ 601 行（plan 预算 250；多出来都在测试） |
| `crates/ui/src/core/mod.rs` | ✅ |
| `crates/app/src/measure_adapter.rs` | ✅ 45 行 |

无差异。

## 3. Phase 2 — UiShell + EditorHost + paint_backend 骨架 ✅

应交付：

| 项 | 实际 |
|---|---|
| `crates/app/src/ui_shell.rs` | ✅ 601 行 |
| `crates/app/src/paint_backend.rs` | ✅ 377 行 |
| `crates/app/src/editor_host.rs` | ✅ 70 行 |
| `App::ui_shell` 字段 | ✅ `crates/app/src/app.rs:87` |
| `render()` 调 `ui_shell.update_frame()` | ✅ `app_renderer.rs:383` |

无差异。

## 4. Phase 3 — status_bar widget + Text 路径 ✅

应交付：

| 项 | 实际 |
|---|---|
| `crates/ui/src/widgets/status_bar.rs` | ✅ 360 行 |
| `paint_backend::drain` 处理 `DrawCmd::Text` | ✅ `paint_backend.rs:37`（不再 panic，真翻译为 atlas + GlyphVertex） |
| `app_renderer::status_bar_bg_vertices` 已删 | ✅ grep 无命中 |
| `app_renderer::status_bar_text_vertices` 已删 | ✅ grep 无命中 |

无差异。

## 5. Phase 4 — search_bar widget + keyboard_focus ✅

应交付：

| 项 | 实际 |
|---|---|
| `crates/ui/src/widgets/search_bar.rs` | ✅ 490 行 |
| `FocusTarget` 枚举 | ✅ `ui_shell.rs:39` |
| `UiShell::keyboard_focus` 字段 | ✅ `ui_shell.rs:53` |
| `UiShell::forward_key` 方法 | ✅ `ui_shell.rs:132` |
| `ui::search_bar::search_bar_bg_vertices` 已删 | ✅ |
| `ui::search_bar::search_bar_cursor_vertices` 已删 | ✅ |
| `app::render_pipeline::search_bar_text_vertices` 已删 | ✅ |
| `crates/ui/src/search_bar.rs` 仅留 `SEARCH_BAR_HEIGHT` 常量 | ✅（实际 9 行注释） |

无差异。

## 6. Phase 5 — scrollbar widget ✅ 主体 / ⚠️ 死 action 残留

应交付：

| 项 | 实际 |
|---|---|
| `crates/ui/src/widgets/scrollbar.rs` | ✅ 471 行 |
| `ScrollbarLayoutPx` + `compute_layout_px` | ✅ `crates/ui/src/scrollbar.rs:11/18` |
| `Widget::as_any`（不可变版） | ✅ `widget.rs:86` |
| 老 NDC `compute_layout / hit_test / generate_vertices / handle_*` | ✅ 全删（scrollbar.rs 从 660 行 → 107 行） |
| 老 `pub struct ScrollbarLayout`（NDC） | ✅ 已删 |
| `App::scrollbar` 字段 | ✅ 已删 |
| `App::scrollbar_dragging` 字段 | ✅ 已删 |

⚠️ **残留**（plan 标注 Phase 9 才删，预警先记）：

```
crates/app/src/actions.rs:72  ScrollbarAction(ScrollbarAction)
crates/app/src/actions.rs:74  SetScrollbarDragging(bool)
crates/app/src/actions.rs:76  UpdateScrollbarState(...)
crates/app/src/actions.rs:80  EndScrollbarDrag
crates/app/src/actions.rs:88  ScrollbarHovered(bool)
```

handler 已是 no-op（`app.rs:1049–1053`），编译告 dead_code。

## 7. Phase 6 — tab_bar 拆分 + widget 化 ❌ **仅完成 30%**

### 7.1 完成项

| 项 | 实际 |
|---|---|
| 文件拆分到 `tab_bar/` 目录 | ✅ `mod / layout / render / hit / state / text` 六个文件齐全 |
| 额外文件 | ✅ 多了 `types.rs / tests.rs`（合理，独立测试入口） |
| `crates/ui/src/widgets/tab_bar.rs` 文件存在 | ✅ 但只有 36 行——**纯空壳**，注释直写"仍走老渲染路径" |

### 7.2 未完成项（plan §Task 3–5 全条未做）

#### 7.2.1 数据结构 ❌

| Plan 要求 | 现状 |
|---|---|
| `TabEntry::rect_px: Rect` 双轨字段 | ❌ 仍是 `rect: [f32; 4]`（NDC） |
| `TabEntry::close_rect_px: Rect` | ❌ 仍是 NDC |
| `TabBarLayout::bar_rect_px / new_tab_rect_px / dropdown_rect_px / fade_*_rect_px / overflow_*_rect_px` | ❌ 全部仍是 NDC `[f32; 4]` |

#### 7.2.2 px 化 API ❌

| Plan 要求 | 现状 |
|---|---|
| `TabBarState::hit_test_px` | ❌ grep 无命中 |
| `TabBarState::on_click_px` | ❌ grep 无命中 |
| `TabBarState::on_mouse_move_px` | ❌ grep 无命中 |
| `TabBarState::to_drawlist` | ❌ grep 无命中 |
| `TabBarAction::OpenContextMenuPx` 变体 | ❌ grep 无命中 |

#### 7.2.3 widget 化 ❌

```rust
// crates/ui/src/widgets/tab_bar.rs（实际 36 行，纯占位）
impl Widget for TabBarWidget {
    fn paint(&self, _ctx: &mut PaintCtx) {
        // Phase 6：仍走老渲染路径
    }
    fn on_event(&mut self, _ev: &Event, _ctx: &mut EventCtx) -> Option<Box<dyn Any>> {
        None
    }
}
```

#### 7.2.4 老路径删除 ❌

| Plan 要求删除 | 现状 |
|---|---|
| `app_renderer::tab_text_vertices`（230 行） | ❌ **仍在 `app_renderer.rs:244`** |
| 老 vertices/text_positions 调用 | ❌ `app_renderer.rs:592` 仍 `extend(workspace.tab_bar_state.vertices(...))` |
| events.rs 改走 widget dispatch | ❌ `events.rs:105/106/325/474` 仍调 `tab_bar_state.on_click / on_mouse_move / hit_test_at`（NDC 版） |

#### 7.2.5 解读

commit `d49972e`（"feat(ui): Phase 4-6 — SearchBarWidget + ScrollbarWidget + tab_bar 拆分优化"）名义合并三阶段，**实际 tab_bar 部分只做了"分文件"**，Phase 6 plan 中真正的核心工作（widget 化 + px 化 + 切换路径 + 删 230 行 tab_text_vertices）都没动。当前 tab_bar 与 popup_menu 的渲染、事件处理、坐标系仍是 1656 行老 NDC 代码（只是分散到 5 个文件）。

## 8. 跨阶段共性问题

### 8.1 workspace 仍持 5 个 UI 字段

```
crates/app/src/workspace.rs:73  context_menu: Option<PopupMenu>
crates/app/src/workspace.rs:74  overflow_menu: Option<PopupMenu>
crates/app/src/workspace.rs:76  tab_bar_state: TabBarState
crates/app/src/workspace.rs:77  sidebar_cfg: SidebarConfig
crates/app/src/workspace.rs:78  sidebar_state: SidebarState
```

- `sidebar_cfg / sidebar_state`：plan 允许双轨保留到 Phase 9（Phase 7 已合并 `21d49b2`，OK）
- `tab_bar_state`：**单轨——只走老的**。这是 Phase 6 未完成的直接后果
- `context_menu / overflow_menu`：Phase 8 才删，目前进度匹配

### 8.2 `popup_menu_text_vertices`（80 行）仍在

```
crates/app/src/app_renderer.rs:151  pub(crate) fn popup_menu_text_vertices(...)
crates/app/src/app_renderer.rs:650/660  vertices.extend(self.popup_menu_text_vertices(...))
```

属 Phase 8 删除目标，符合预期。

### 8.3 dead_code 警告（10 条）

主要来自 Phase 5 的 ScrollbarAction 系列、`traffic_light_inset` 等。Phase 9 收尾时统一处理。

## 9. 风险与建议

### 9.1 **强烈建议在执行后续阶段之前，把 Phase 6 真正补完**

Phase 7（SidebarWidget）已经合并（`21d49b2`），意味着现状是：

- `sidebar` 走 widget + ui_shell.dispatch（px 形态）
- `tab_bar` 走 workspace.tab_bar_state.on_click（NDC 形态）

**两套语义并存**，events.rs 里出现"sidebar 走新路径、tab_bar 走老路径"的混合分支，未来调试与扩展都会很别扭。

### 9.2 Phase 8 严重依赖 Phase 6 真完工

`PopupMenu::overflow` 需要 `dropdown_rect_px`；`OpenContextMenuPx` 需要 px 形态 anchor。**Phase 6 不补完，Phase 8 无法干净落地**。

### 9.3 最小补丁清单（提给"补完 Phase 6"的实施者）

按 `phase6.md` 的 Task 编号：

1. **Task 3** — layout 加 px 字段（`rect_px / close_rect_px / bar_rect_px / new_tab_rect_px / dropdown_rect_px / fade_*_rect_px / overflow_*_rect_px`），`layout_tabs` 内部填充
2. **Task 4** — TabBarState 加 `hit_test_px / on_click_px / on_mouse_move_px / to_drawlist`；TabBarAction 加 `OpenContextMenuPx { tab_index, anchor_px }`
3. **Task 5** — UiShell 注册真 widget（`enable_widget_paint`）；删 `app_renderer::tab_text_vertices`；events.rs 切到 ui_shell.dispatch；保留 workspace.tab_bar_state 双轨直到 Phase 9

完工标准：`grep "fn tab_text_vertices" crates/` 无命中；`grep "tab_bar_state.on_click\|tab_bar_state.hit_test_at\|tab_bar_state.on_mouse_move"` 无命中。

## 10. 测试覆盖速览

| crate | 通过 | 失败 | 备注 |
|---|---|---|---|
| edit-plus-ui | 275 | 0 | core/widgets/tab_bar/sidebar 全套 |
| edit-plus-app | 548 | 0 | 含 ui_shell / paint_backend / measure_adapter / sidebar_widget |
| edit-plus-render lib-test | — | 编译失败 | wgpu git rev 与 lib-test 不兼容；非本重构引入；主程序 build 与运行不受影响 |

**结论**：测试基础设施健康；功能上看 Phase 1–5 + 7 + 7.5 plan 都已落地，**唯独 Phase 6 主体（widget 化）需要补**。
