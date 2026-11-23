# Plan：事件流清理与计算冗余消除

> 对应原方案 Phase 4-5。依赖 `plans-ui-merge.md` 完成后再执行（需要 PopupMenu 和 Sidebar 已合并到 widgets/）。

---

## 背景

代码验证确认：Dock 已实现完整递归树派发（hit-test 路由、capture 支持、MouseMove 广播）。"事件分发硬编码"的说法不成立。真正的问题是：

1. **events.rs 中的 Workaround 代码** — 为了绕过 `TabBarWidget` 的行为缺陷而存在的二次 dispatch + 手动 hit_test
2. **TabBarState 双重计算** — workspace 和 widget 各自独立计算全量布局
3. **render_pipeline.rs 中的复制粘贴** — 6 处相同模式

---

## Step 1：消除 TabBar 双重布局（3 小时）

### 现状

- `workspace::update_tab_layout`（workspace.rs:711-808）：在 action handler 中计算 TabBarLayout → 写入 `UiShell.tab_bar_state`
- `TabBarWidget::set_tabs_input`（tab_bar.rs:76）：在每帧 `update_frame` 时再次计算 → 写入 widget 内部 state
- 两套 `TabBarState` 实例，布局存两份

### 附带 Bug

`tick_scroll_animation`（workspace.rs:830）推进 `tab_scroll_offset` 后不调用 `update_tab_layout`，导致 `UiShell.tab_bar_state` 中的布局使用的是旧偏移量，而 widget 内的布局使用的是新偏移量——两者不同步。

### 操作

| # | 步骤 |
|---|------|
| 1.1 | 移除 `UiShell.tab_bar_state` 字段 |
| 1.2 | app.rs 中 3 个读取布局的 action handler（OpenPopupOverflow:965、ScrollTabLeft:1033、ScrollTabRight:1047）改为从 Dock child 中获取 TabBarWidget 的布局 |
| 1.3 | 删除 `workspace::update_tab_layout` 中的布局计算调用，仅保留 scroll_offset 和 animation 状态管理 |
| 1.4 | `max_scroll` 改为使用 `tab_bar::layout::max_tab_scroll()` 独立函数（layout.rs:176），无需完整布局 |
| 1.5 | 让 `TabBarWidget::set_tabs_input` 成为唯一布局计算入口 |

---

## Step 2：消除 events.rs 的 TabBar 二次 dispatch（2 小时）

### 根因

`TabBarWidget::on_event` 对 `MouseMove` 返回 `SwitchTab(idx)` 而非 hover 语义的 action。events.rs:148-184 的二次 dispatch + 手动 hit_test 是为了：

1. 重新 dispatch 以触发 `TabBarState::on_mouse_move_px` 的副作用（更新 hovered_index）
2. 手动 hit_test 产生正确的 `HoverTab(idx)` / `HoverTab(None)` action

### 操作

| # | 步骤 |
|---|------|
| 2.1 | `TabBarWidget::on_event` 对 `MouseMove` 改为返回 `WidgetAction::TabBar(TabBarAction::HoverTab(idx_opt))`（新增变体或复用现有），不再返回 `SwitchTab` |
| 2.2 | `translate_tab_action` 中新增 `HoverTab` → `AppAction::HoverTab` 映射 |
| 2.3 | 删除 events.rs:148-184 整段二次 dispatch + 手动 hit_test |
| 2.4 | 确认 `SetCursor` 在 Dock dispatch 中已正确设置（`ctx.cursor_hint` 在 tab_bar.rs:113-124 已处理） |

---

## Step 3：简化 Sidebar hover 并行路径（1 小时）

### 现状

events.rs:115-120 中 `sidebar_on_mouse_move` 调用独立于 Dock 派发链路，原因是 `SidebarPersistent` 状态只注入 widget（inject_persistent）但从不从 widget 同步回来。

### 操作

| # | 步骤 |
|---|------|
| 3.1 | 在 `UiShell::update_widget_state` 中，widget 处理后调用 `steal_persistent()` 同步回 `ui_shell.sidebar_persistent` |
| 3.2 | 删除 events.rs:115-120 的并行 `sidebar_on_mouse_move` 调用 |
| 3.3 | 确认 SidebarWidget 已在 Dock 派发中正确接收所有 MouseMove（含 overlay 场景） |

---

## Step 4：Sidebar per-frame 分配优化（1 小时）

`widgets/sidebar.rs:286-291` 每帧 `tabs.iter().map(|t| ListItem { label: t.title.clone(), ... })` 产生高频 String clone。

| # | 步骤 |
|---|------|
| 4.1 | 引入脏检查：仅在 tabs 列表变化时重建 `Vec<ListItem>` |
| 4.2 | 可选：将 `label: String` 改为 `label: Arc<str>` 共享所有权 |

---

## Step 5：render_pipeline.rs 代码去重（2 小时）

### 5.1 提取 whitespace cluster advance（节省 6 处重复）

`render_pipeline.rs` 中 578-579, 588-589, 660-661, 734-736, 804-805, 846-847 行有相同模式：

```rust
let is_ws = is_whitespace_cluster(&line_bytes[c.byte_range.clone()]);
let adv = if is_ws { ws_cluster_advance(&line_bytes[c.byte_range.clone()], char_width) } else { c.advance.max(1.0) };
```

提取为：

```rust
fn cluster_advance(cluster: &GlyphCluster, line_bytes: &[u8], char_width: f32) -> (bool, f32)
```

`layout.rs:27-28` 中类似模式也一并替换。

### 5.2 消除 tab_infos 双 clone

`app_renderer.rs:328,342` 中 `tab_infos` 每帧 clone 两次（分别给 set_tabs_input 和 set_sidebar_input）。改为通过引用传递或 share。

### 5.3 sidebar `item_h = 28.0` vs `ROW_H = 24.0` 不一致

`sidebar.rs:456,979` 中硬编码 28.0 与常量 ROW_H=24.0 不一致，确认正确值后使用对应常量。

---

## 验证

- `cargo test` 全通过
- 手动测试清单：
  - [ ] Tab 上 hover 切换光标样式
  - [ ] Sidebar 自动隐藏/显示（鼠标移入/移出边缘）
  - [ ] Tab 栏滚动动画流畅
  - [ ] 右键菜单定位正确
  - [ ] Sidebar 文件列表项点击选中正常

## 工作量

~9 小时（约 1.5 天）。建议 Step 1-3 连续做（高度耦合），Step 4-5 可独立。
