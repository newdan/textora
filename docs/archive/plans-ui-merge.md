# Plan：UI 双层架构合并

> 对应原方案 Phase 3。依赖 `plans-cleanup.md` 中的常量文件（`ui/src/constants.rs`）已就位。

---

## 背景

`crates/ui/src/` 存在旧模块和新 Widget 并存的双层结构：

| 旧模块 | 新 Widget | 关系 |
|--------|----------|------|
| `sidebar.rs` (1646行) | `widgets/sidebar.rs` (1408行) | Widget 包装旧 SidebarState |
| `status_bar.rs` (~100行) | `widgets/status_bar.rs` | Widget 包装旧类型 |
| `title_bar.rs` (~150行) | `widgets/title_bar.rs` | Widget 包装旧类型 |
| `popup_menu.rs` (~400行) | `widgets/popup_menu.rs` | Widget 薄包装 |
| `scrollbar.rs` (162行) | `widgets/scrollbar.rs` | Widget 包装旧纯函数 |

目标：将旧模块的类型/状态/逻辑合并到 `widgets/` 下，消除顶层旧文件。

---

## 合并顺序（由简到难）

### Step 1：Scrollbar（45 分钟）

旧模块已是纯函数 `compute_layout_px` + `ScrollbarLayoutPx`，最简单。

- 合并到 `widgets/scrollbar.rs`
- 旧文件改为 `pub use widgets::scrollbar::*;` 兼容 re-export
- 更新所有外部 `use ui::scrollbar::` 引用后删除旧文件

### Step 2：StatusBar（30 分钟）

旧模块只有 `StatusBarInput` + `build_text`，无状态机。

- 合并到 `widgets/status_bar.rs`，删除旧文件

### Step 3：TitleBar（1 小时）

- 合并到 `widgets/title_bar.rs`，删除旧文件

### Step 4：PopupMenu（3 小时）

旧模块有状态管理 + action 枚举。需要拆子模块：

```
widgets/popup_menu/
├── mod.rs      # Widget trait 实现 + 渲染
├── types.rs    # PopupMenu, PopupMenuAction, PopupMenuItem, ContextMenuAction 等
└── state.rs    # 状态管理逻辑
```

- 更新 `tab_bar/mod.rs` 中 popup_menu 的 re-export 链（让其直指 `widgets::popup_menu`）
- 删除旧 `ui/src/popup_menu.rs`

### Step 5：Sidebar（1.5 天，最复杂）

3054 行总量。目标结构：

```
widgets/sidebar/
├── mod.rs        # Widget trait 实现
├── state.rs      # SidebarState + SidebarAction
├── layout.rs     # SidebarLayoutItem 计算
├── paint.rs      # 绘制逻辑（整合 widget 动画层 + 旧 chrome 绘制）
├── types.rs      # SidebarInput, SidebarCfg 等
└── persistent.rs # SidebarPersistent
```

关键约束：
- `SidebarState::update_layout` **被渲染使用**（计算全部元素的几何布局），不能删除
- 需保留 `SidebarPersistent` 机制（`inject_persistent` 在生产中使用，用于跨帧保持隐藏/显示状态）
- `steal_state()` / `inject_state()` 按 `plans-cleanup.md` 已在 B2.8 删除
- 更新 app 层所有引用路径

### Step 6：整理 tab_bar re-export（1 小时）

`tab_bar/mod.rs` 中 `pub use crate::popup_menu::{...};` 改为让调用者直接引用 `ui::widgets::popup_menu::...`。

---

## 验证

- 每步 `cargo check --all-targets` 零错误 + `cargo test` 全通过
- 手动测试 checklist：
  - [ ] Sidebar 展开/折叠/拖拽调整宽度
  - [ ] 文件列表点击选中/关闭
  - [ ] 右键菜单（context menu + overflow menu）
  - [ ] Tab 切换 / 关闭 / 拖拽排序
  - [ ] 搜索栏弹出/输入/关闭

## 工作量

~2-3 天。建议按 Step 1-6 顺序逐个 PR 提交，每个 Step 独立可合。

## 原子化 PR

| PR | 内容 | 预计 |
|----|------|------|
| PR1 | Scrollbar 合并 | 45min |
| PR2 | StatusBar + TitleBar 合并 | 1.5h |
| PR3 | PopupMenu 合并 | 3h |
| PR4 | Sidebar 合并 | 1.5d |
| PR5 | tab_bar re-export 整理 | 1h |
