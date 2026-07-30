# UI 架构重构计划 (相对坐标与 WidgetId)

**目标**：解决目前 UI 骨架强依赖绝对坐标计算容易出错的问题，以及焦点路由通过强转造成的耦合问题。为保证极致性能，重构保持扁平化结构，增加偏移量推入和全局唯一 WidgetId 设计。

## 阶段 1：底层协议扩展 (接口与数据结构)
*目标：建立偏移量透传机制与 WidgetId 定义，不影响现有业务逻辑*
- `crates/ui/src/core/widget.rs`:
  - 引入 `WidgetId` 包装类型 (`#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq)] pub struct WidgetId(pub u64);`)。
  - 在 `Widget` trait 中增加 `fn id(&self) -> Option<WidgetId> { None }` 方法。
  - 给 `PaintCtx` 新增 `pub offset: (f32, f32)`。
- `crates/ui/src/core/paint.rs`:
  - 修改 `DrawList::fill`, `text`, `clip` 等辅助方法，使其在收集 DrawCmd 时自动加上 `offset.0` 和 `offset.1`。
- `crates/ui/src/core/dock.rs`:
  - 修改 `DockChild` 结构体，增加 `pub layout_rect: Rect`。
  - `Dock::layout` 遍历分配子节点区域时，顺便把分配到的 `child_rect` 缓存在 `layout_rect` 里。

## 阶段 2：事件分发支持相对坐标
*目标：在 Dock 与 UiShell 层，完成从绝对坐标到相对坐标的转换下发*
- `crates/ui/src/core/dock.rs`:
  - 修改 `Dock::dispatch`。针对鼠标事件 (`MouseMove`, `MouseDown`, `MouseUp`, `Wheel`)，利用缓存的 `child.layout_rect` 对坐标进行相减 `(px - rect.x, py - rect.y)`，生成一个局部坐标系的 `Event` 再投递给 child。
- `crates/app/src/ui_shell.rs`:
  - `UiShell::dispatch` 对于 `overlays` 的分发也应用类似的偏移量减法。

## 阶段 3：迁移简单组件到相对坐标
*目标：验证底层相对坐标系可行性，转换相对简单的 Widget*
- **SearchBarWidget** (`crates/ui/src/widgets/search_bar.rs`)
- **StatusBarWidget** (`crates/ui/src/widgets/status_bar.rs`)
- **TabBarWidget** (`crates/ui/src/widgets/tab_bar.rs`)
- **ScrollbarWidget** (`crates/ui/src/widgets/scrollbar.rs`)
- **改动方式**：
  - `set_rect` 实现不再缓存屏幕绝对 `x, y`，只关心传进来的 `rect.w` / `rect.h`。
  - `hit(px, py)` 实现均改为判断局部大小 (`0 <= px <= w`, `0 <= py <= h`)。
  - `paint` 调用中，如果内部要画子元素，坐标直接从 0,0 开始算起。

## 阶段 4：迁移复杂组件到相对坐标
*目标：完成最具挑战性、状态流转最复杂的 Widget 迁移*
- **SidebarWidget** (`crates/ui/src/widgets/sidebar.rs` & `sidebar.rs`):
  - 重构内部的状态模块 `SidebarState`，所有如 `menu_btn_rect`、`bg_rect` 的高度写死计算，全改作局部坐标起始。
  - 修复 `StartResize` 和边框拖动逻辑，使其适应局部坐标系（计算宽度时不要依赖屏幕绝对 `px` 减偏置）。
- **VerticalListWidget** (`crates/ui/src/widgets/list.rs`):
  - 列表元素的局部命中检测 `hit_row` 重算，内部元素排版摒弃父容器绝对 y 坐标。
- **PopupMenu** (`crates/ui/src/widgets/popup_menu.rs`):
  - 改造弹出菜单在相对坐标体系下的展现与命中判定。

## 阶段 5：解耦键盘焦点系统与消除 Downcast
*目标：废除白名单 `FocusTarget` 枚举，用泛用的 `WidgetId` 树形遍历进行键盘事件路由*
- `crates/app/src/ui_shell.rs`:
  - 修改 `keyboard_focus` 字段类型为 `Option<WidgetId>`。
  - 彻底移除丑陋的硬编码强转：`child.widget.as_any_mut().downcast_mut::<SearchBarWidget>()`。
- `crates/ui/src/core/dock.rs`:
  - 修改 `Dock::dispatch`：遇到 `KeyDown` 事件时，扫描所有子节点，将事件精准派发给 `id() == shell_focus_id` 的 Widget。
