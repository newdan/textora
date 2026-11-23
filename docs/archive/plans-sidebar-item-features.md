# Sidebar Item 增强功能方案

## 需求概述

1. **Hover 关闭按钮**：workspace item hover 时，在右侧显示关闭按钮（非 pinned item）
2. **Pinned 锁定图标**：pinned item 前面显示锁定图标

## 涉及文件

- `crates/ui/src/core/widget.rs` - 新增 `WidgetAction::List` 变体
- `crates/ui/src/widgets/list.rs` - VerticalListWidget 核心逻辑
- `crates/ui/src/widgets/sidebar.rs` - SidebarWidget 使用层
- `crates/ui/src/sidebar.rs` - 新增 `SidebarAction::CloseTab` 变体

## 详细设计

### 1. WidgetAction 新增变体 (`widget.rs`)

```rust
pub enum WidgetAction {
    Sidebar(crate::sidebar::SidebarAction),
    TabBar(crate::tab_bar::TabBarAction),
    Scrollbar(crate::widgets::scrollbar::ScrollbarAction),
    SearchBar(crate::widgets::search_bar::SearchBarAction),
    Popup(crate::PopupOutcome),
    List(crate::widgets::list::ListAction),  // 新增
    Consumed,
}
```

### 2. SidebarAction 新增变体 (`sidebar.rs`)

```rust
pub enum SidebarAction {
    // ...existing variants...
    CloseTab(usize),  // 新增：关闭第 index 个 workspace tab
}
```

### 3. ListItem 结构修改 (`list.rs`)

```rust
#[derive(Clone, Debug, Default)]
pub struct ListItem {
    pub label: String,
    pub kind: ListItemKind,
    pub indicator: ListItemIndicator,
    pub pinned: bool,  // 新增：是否 pinned（控制锁定图标和关闭按钮是否显示）
}
```

**说明**：
- `pinned`：控制行首显示锁定图标，同时 pinned item 不显示关闭按钮

### 4. ListAction 新增变体 (`list.rs`)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListAction {
    Selected(usize),
    HoverChanged(Option<usize>),
    CloseRequested(usize),  // 新增：点击关闭按钮
}
```

### 5. 布局常量 (`list.rs`)

```rust
/// 锁定图标尺寸（逻辑像素）
const LOCK_ICON_SIZE_LOGICAL: f32 = 10.0;
/// 锁定图标右边距（逻辑像素）
const LOCK_ICON_MARGIN_LOGICAL: f32 = 6.0;
/// 关闭按钮尺寸（逻辑像素）
const CLOSE_BTN_SIZE_LOGICAL: f32 = 16.0;
/// 关闭按钮左边距（逻辑像素）
const CLOSE_BTN_MARGIN_LOGICAL: f32 = 4.0;
```

### 6. paint 方法修改 (`list.rs`)

```rust
fn paint(&self, ctx: &mut PaintCtx) {
    // ...existing code...

    for (i, item) in self.items.iter().enumerate() {
        let row_rect = self.item_rect(i, dpi);

        match item.kind {
            ListItemKind::Separator => { /* ...existing code... */ }
            ListItemKind::Header | ListItemKind::Normal => {
                // ...existing hover/active bg code...

                let mut text_x = row_rect.x + pad_x;

                // 1) 锁定图标（pinned item）
                if item.pinned {
                    let lock_size = LOCK_ICON_SIZE_LOGICAL * dpi;
                    let lock_margin = LOCK_ICON_MARGIN_LOGICAL * dpi;
                    let lock_x = text_x;
                    let lock_y = row_rect.y + (row_rect.h - lock_size) * 0.5;

                    let mut lock_fg = self.style.item_fg;
                    lock_fg[3] *= alpha * 0.6;
                    // 锁体
                    ctx.list.fill(
                        Rect::new(lock_x, lock_y, lock_size, lock_size * 0.7),
                        lock_fg,
                    );
                    // 锁扣（上半部分弧，简化为矩形）
                    ctx.list.fill(
                        Rect::new(lock_x + lock_size * 0.2, lock_y - lock_size * 0.3,
                                  lock_size * 0.6, lock_size * 0.3),
                        lock_fg,
                    );

                    text_x += lock_size + lock_margin;
                }

                // 2) 关闭按钮区域（始终预留空间，避免 hover 时文字跳变）
                let close_btn_w = if item.pinned {
                    0.0
                } else {
                    CLOSE_BTN_SIZE_LOGICAL * dpi + CLOSE_BTN_MARGIN_LOGICAL * dpi
                };

                // 3) 文字
                let dot_extra = if matches!(item.indicator, ListItemIndicator::Dot) {
                    dot_r * 2.0 + 4.0 * dpi
                } else {
                    0.0
                };
                let label_max_w = (row_rect.w - pad_x * 2.0 - close_btn_w - dot_extra
                    - (text_x - row_rect.x - pad_x)).max(0.0);
                let label = truncate_title_by_width(&item.label, label_max_w, font_size);

                let baseline = row_rect.y + row_rect.h * 0.5 + font_size * 0.35;
                let mut fg = self.style.item_fg;
                fg[3] *= alpha;
                ctx.list.text(text_x, baseline, font_size, fg, &label);

                // ...existing indicator code...

                // 4) 关闭按钮（hover 时绘制）
                if !item.pinned && Some(i) == self.hovered_index {
                    let btn_size = CLOSE_BTN_SIZE_LOGICAL * dpi;
                    let btn_x = row_rect.x + row_rect.w - pad_x - btn_size;
                    let btn_y = row_rect.y + (row_rect.h - btn_size) * 0.5;

                    // 关闭按钮 hover 背景
                    let btn_bg = [0.3, 0.3, 0.3, 0.5 * alpha];
                    ctx.list.fill(
                        Rect::new(btn_x, btn_y, btn_size, btn_size),
                        btn_bg,
                    );

                    // ✕ 文字
                    let cross_font_size = 11.0 * dpi;
                    let mut cross_fg = [0.8, 0.8, 0.8, alpha];
                    let cross_baseline = btn_y + btn_size * 0.5 + cross_font_size * 0.35;
                    ctx.list.text(
                        btn_x + btn_size * 0.15,
                        cross_baseline,
                        cross_font_size,
                        cross_fg,
                        "✕",
                    );
                }
            }
        }
    }
}
```

### 7. hit_close_btn 方法 (`list.rs`)

实时计算关闭按钮矩形，无需缓存字段。只在鼠标点击时调用，开销可忽略。

```rust
impl VerticalListWidget {
    /// 检测点击是否命中关闭按钮（仅非 pinned 且当前 hover 的行）
    pub(crate) fn hit_close_btn(
        &self, px: f32, py: f32, scroll_offset: f32, dpi: f32,
    ) -> Option<usize> {
        let shifted_py = py + scroll_offset;
        for (i, item) in self.items.iter().enumerate() {
            if item.pinned { continue; }
            if Some(i) != self.hovered_index { continue; }
            let row_rect = self.item_rect(i, dpi);
            let btn_size = CLOSE_BTN_SIZE_LOGICAL * dpi;
            let pad_x = self.style.pad_x_logical * dpi;
            let btn_x = row_rect.x + row_rect.w - pad_x - btn_size;
            let btn_y = row_rect.y + (row_rect.h - btn_size) * 0.5;
            let btn_rect = Rect::new(btn_x, btn_y, btn_size, btn_size);
            if btn_rect.contains(px, shifted_py) {
                return Some(i);
            }
        }
        None
    }
}
```

### 8. on_event 方法修改 (`list.rs`)

```rust
fn on_event(&mut self, ev: &Event, ctx: &mut EventCtx) -> Option<WidgetAction> {
    match ev {
        Event::MouseDown { px, py, button: MouseButton::Left } => {
            // 优先检测关闭按钮
            if let Some(idx) = self.hit_close_btn(*px, *py, self.scroll_offset, ctx.dpi) {
                return Some(WidgetAction::List(ListAction::CloseRequested(idx)));
            }

            // 原有逻辑：检测行点击
            if let Some(idx) = self.hit_row(*px, *py, self.scroll_offset, ctx.dpi) {
                return Some(WidgetAction::List(ListAction::Selected(idx)));
            }
            None
        }
        Event::MouseMove { px, py } => {
            // ...existing hover logic...
        }
        _ => None,
    }
}
```

### 9. SidebarWidget 修改 (`sidebar.rs`)

#### 9.1 构建 ListItem

```rust
let items: Vec<ListItem> = self.tabs.iter().map(|t| ListItem {
    label: t.title.clone(),
    kind: ListItemKind::Normal,
    indicator: if t.is_dirty { ListItemIndicator::Dot } else { ListItemIndicator::None },
    pinned: t.pinned,
}).collect();
self.list.set_items(items);
```

#### 9.2 处理 ListAction（MouseDown 分支）

将现有 `MouseDown` 流程中第 3 步（hit test list items）改为委托给 `self.list.on_event()`：

```rust
Event::MouseDown { px, py, button } if *button == MouseButton::Left => {
    // ...existing: menu dispatch, edge resize, hit_test_px...

    // 3) List item hit test & close button
    if let Some(layout) = self.state.current_layout() {
        self.list.set_scroll_offset(self.list_scroll_offset());
        if let Some(action) = self.list.on_event(ev, ctx) {
            match action {
                WidgetAction::List(ListAction::Selected(sorted_idx)) => {
                    let ws_idx = self.tab_index_map.get(sorted_idx).copied().unwrap_or(sorted_idx);
                    return Some(WidgetAction::Sidebar(SidebarAction::SwitchTab(ws_idx)));
                }
                WidgetAction::List(ListAction::CloseRequested(sorted_idx)) => {
                    let ws_idx = self.tab_index_map.get(sorted_idx).copied().unwrap_or(sorted_idx);
                    return Some(WidgetAction::Sidebar(SidebarAction::CloseTab(ws_idx)));
                }
                _ => {}
            }
        }
    }

    None
}
```

## 绘制效果示意

### Pinned Item
```
┌─────────────────────────────────────┐
│ 🔒 file.rs                    *    │
│    ^^^^                        ^    │
│    锁定图标                    dirty │
└─────────────────────────────────────┘
```

### Hover Item (非 pinned)
```
┌─────────────────────────────────────┐
│ main.rs                       ✕    │
│                               ^^    │
│                             关闭按钮 │
└─────────────────────────────────────┘
```

### Normal Item (未 hover, 非 pinned)
```
┌─────────────────────────────────────┐
│ lib.rs                              │
│    ^^^^ 右侧预留关闭按钮空间但不可见  │
└─────────────────────────────────────┘
```

## 边界情况处理

### 1. 行宽不足
- 优先保证锁定图标显示
- 文字截断时为关闭按钮预留空间（始终预留）
- 关闭按钮始终可见（即使文字被严重截断）

### 2. Pinned + Dirty 状态
```
┌─────────────────────────────────────┐
│ 🔒 config.toml                 *   │
│    ^^^^                        ^    │
│    锁定图标                    dirty │
└─────────────────────────────────────┘
```

### 3. 滚动场景
- hit_close_btn 实时计算关闭按钮矩形，接收 scroll_offset 参数进行偏移
- paint 中的 row_rect 已包含 scroll_offset（通过 ctx.list.offset 处理）

### 4. 快速 Hover 切换
- 当前实现：立即显示/隐藏关闭按钮
- 文字宽度始终预留，不会跳变
- 后续优化：可加 100ms 延迟，避免关闭按钮频繁闪烁

## 测试用例

### list.rs 测试

```rust
#[test]
fn pinned_item_paints_lock_icon() {
    // 验证 pinned item 绘制锁定图标
}

#[test]
fn non_pinned_item_no_lock_icon() {
    // 验证非 pinned item 不绘制锁定图标
}

#[test]
fn hover_shows_close_button() {
    // 验证 hover 时绘制关闭按钮
}

#[test]
fn pinned_item_no_close_button_on_hover() {
    // 验证 pinned item hover 时不显示关闭按钮
}

#[test]
fn click_close_button_returns_close_requested() {
    // 验证点击关闭按钮返回 CloseRequested
}

#[test]
fn click_outside_close_button_returns_selected() {
    // 验证点击关闭按钮外区域返回 Selected
}

#[test]
fn close_button_space_reserved_when_not_hovered() {
    // 验证非 hover 状态下文字为关闭按钮预留空间（文字不跳变）
}
```

### sidebar.rs 测试

```rust
#[test]
fn close_tab_on_non_pinned_item() {
    // 验证关闭非 pinned tab
}

#[test]
fn pinned_item_has_lock_in_list_item() {
    // 验证 pinned tab 对应的 ListItem 设置了 pinned 字段
}
```

## 实施步骤

### Phase 1: 类型定义
1. 新增 `WidgetAction::List(ListAction)` (`widget.rs`)
2. 新增 `SidebarAction::CloseTab(usize)` (`sidebar.rs`)
3. 修改 `ListItem` 结构，添加 `pinned` 字段 (`list.rs`)
4. 添加 `ListAction::CloseRequested` 变体 (`list.rs`)

### Phase 2: list.rs 核心逻辑
1. 添加布局常量（LOCK_ICON_SIZE 等）
2. 实现 paint 中的锁定图标绘制
3. 实现 paint 中的关闭按钮绘制（文字 "✕"）
4. 实现 hit_close_btn 方法（实时计算）
5. 修改 on_event：MouseDown 返回 ListAction 变体
6. 始终为关闭按钮预留空间
7. 添加单元测试

### Phase 3: sidebar.rs 集成
1. 修改 ListItem 构建逻辑（传入 `pinned`）
2. 将 MouseDown 的 list hit test 改为委托 `self.list.on_event()`
3. 处理 `ListAction::CloseRequested` → `SidebarAction::CloseTab`
4. 添加测试

### Phase 4: 上层事件处理
1. 在 `events.rs` 或 workspace 中处理 `SidebarAction::CloseTab`
2. 验证完整流程

### Phase 5: 验证
1. 运行 `cargo check -p edit-plus-app`
2. 运行 `cargo test -p edit-plus-app --lib`
3. 视觉验证

## 相关参考

- TabBar 关闭按钮实现：`crates/ui/src/tab_bar/state.rs`
- SidebarWidget 现有逻辑：`crates/ui/src/widgets/sidebar.rs`
- WidgetAction 定义：`crates/ui/src/core/widget.rs`
- SidebarAction 定义：`crates/ui/src/sidebar.rs`
