# button + ListWidget 基础模块设计

## 概述

在 `crates/ui/src/widgets/` 下新增两个基础模块：

- **button.rs**：通用的 Button Widget，支持可选的 icon 和 text label
- **list.rs**：在现有 `VerticalListWidget` 上扩展为 `ListWidget`，增加 `Orientation`（Vertical/Horizontal）和新 item 字段

目标：抽取重复的按钮绘制逻辑，用一个 ListWidget 覆盖 workspace items、menu items、search bar 按钮行三个场景。

---

## 1. button.rs — Button Widget

### 类型

```rust
pub struct ButtonStyle {
    pub font_size_logical: f32,
    pub pad_x_logical: f32,
    pub fg: [f32; 4],
    pub hover_bg: [f32; 4],
}

pub struct Button {
    rect: Rect,
    icon: Option<String>,
    icon_size_logical: f32,
    text: Option<String>,
    style: ButtonStyle,
    hovered: bool,
    is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ButtonAction {
    Click,
}
```

### 行为

- `set_rect`：存储 rect，无特殊 layout 逻辑
- `paint`：按 hover/active/normal 优先级选背景色（圆角常量 4dp），有 icon 则 `draw_icon`，有 text 则 `text_shaped`（icon 在左，文字在 icon 右侧）
- `hit`：rect.contains
- `on_event`：MouseMove 更新 hovered；MouseDown(Left) 返回 `ButtonAction::Click`
- `is_active`：由调用方通过 setter 控制（如 settings 按钮打开菜单时保持按下态）

### 尺寸

调用方在 `set_rect` 中给定 rect，Button 本身不做 measure。外部布局容器决定尺寸。

---

## 2. list.rs — ListWidget

### 改动策略

在现有 `VerticalListWidget` 上直接修改，重命名为 `ListWidget`。

### 新增：Orientation

```rust
pub enum Orientation {
    Vertical,
    Horizontal,
}
```

### 新增：ListItem 字段

```rust
pub struct ListItem {
    // 现有
    pub label: String,
    pub kind: ListItemKind,
    pub indicator: ListItemIndicator,
    pub pinned: bool,
    // 新增
    pub icon: Option<String>,          // icon 名字
    pub extra_label: Option<String>,   // 行尾辅助文字
    pub is_active: bool,               // ← 之前在外部管理，现在搬进来
    pub closeable: bool,               // ← 之前由 !pinned 推导，现在显式
}
```

### Orientation 影响点

| 维度 | Vertical | Horizontal |
|------|----------|------------|
| item_rect | Rect(x, y+pad+i*h, w, h) | Rect(x+pad+i*w, y, w, h) |
| scroll 轴 | Y | X |
| Separator 绘制 | 横线 | 竖线 |
| close button | 行右侧 | 行右侧（不变） |
| pin bar | 行左侧竖线 | 行顶横线 |

### 新增字段的 paint 规则

- `icon`：画在 label 左侧
- `extra_label`：画在行右侧（close button 左边）
- `closeable && hovered`：显示 X 按钮
- `pinned`：画左侧竖线（pin 标记），与 closeable 独立——两者可同时出现

### 滚动

Horizontal 模式下 scroll_offset 沿 X 轴偏移 item_rect。

### 公开 API 变化

- `VerticalListWidget` → `ListWidget`
- `ListStyle` 增加 `item_w_logical`（Horizontal 模式的列宽，Vertical 模式忽略）
- 构造函数增加 `orientation: Orientation` 参数

### 向后兼容

`VerticalListWidget` 的所有现有测试继续通过，因为 Vertical 模式下行为不变（item_w 用 rect.w，忽略 item_w_logical）。

---

## 3. 调用方改动

### Sidebar（workspace items）

用 `ListWidget` + `Orientation::Vertical` 替换现有 `VerticalListWidget`。ListItem 设置 pinned/closeable/indicator/extra_label（如需要）。

### PopupMenu（menu items）

用 `ListWidget` + `Orientation::Vertical`。ListItem 设置 icon(optional)/is_active。Separator 用 `kind=Separator`。

### SearchBar 按钮行

用 `ListWidget` + `Orientation::Horizontal`，每个 item 设 icon。一行 icon 按钮。

### StatusBar 中的按钮

用 `Button` 替换当前 inline 绘制逻辑（如 eye toggle）。

---

## 4. 测试

- Button：paint (icon only, text only, both, hover bg, active bg)、hit、click、hover 状态切换
- ListWidget：现有 VerticalListWidget 测试全部保留，新增 Horizontal 方向测试（item_rect 排列、scroll、hit）
