# Sidebar Item 增强功能 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 sidebar 列表中为 pinned item 显示锁定图标，为非 pinned item 提供 hover 关闭按钮。

**Architecture:** 修改 VerticalListWidget（通用列表 primitive）的 paint/on_event 来绘制锁定图标和关闭按钮，通过 ListAction::CloseRequested 向上传递事件。SidebarWidget 负责将 TabInfo 映射为 ListItem，并将 CloseRequested 翻译为 SidebarAction::CloseTab。顶层 events.rs 将 CloseTab 映射为已有的 AppAction::CloseTab。

**Tech Stack:** Rust,现有 custom UI framework(cosmic-text based)

---

### Task 1: 新增类型定义

**Files:**
- Modify: `crates/ui/src/core/widget.rs`
- Modify: `crates/ui/src/sidebar.rs`
- Modify: `crates/ui/src/widgets/list.rs`

- [ ] **Step 1: 在 WidgetAction 中新增 List 变体**

编辑 `widget.rs`，在 `Consumed` 之前插入：

```rust
pub enum WidgetAction {
    // ...existing variants...
    Popup(crate::PopupOutcome),
    List(crate::widgets::list::ListAction),
    /// 事件已消费但无需 AppAction（如 hover 更新）
    Consumed,
}
```

- [ ] **Step 2: 在 SidebarAction 中新增 CloseTab 变体**

编辑 `sidebar.rs`，在 `SwitchTab(usize)` 之后插入：

```rust
pub enum SidebarAction {
    SwitchTab(usize),
    CloseTab(usize),  // 新增：关闭第 index 个 workspace tab
    NewDocument,
    // ...rest of variants...
}
```

- [ ] **Step 3: 在 ListItem 中新增 pinned 字段**

编辑 `list.rs`：

```rust
#[derive(Clone, Debug, Default)]
pub struct ListItem {
    pub label: String,
    pub kind: ListItemKind,
    pub indicator: ListItemIndicator,
    pub pinned: bool,
}
```

- [ ] **Step 4: 在 ListAction 中新增 CloseRequested 变体**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListAction {
    Selected(usize),
    HoverChanged(Option<usize>),
    CloseRequested(usize),
}
```

- [ ] **Step 5: 编译验证**

```bash
cargo check -p edit-plus-ui 2>&1
```

- [ ] **Step 6: Commit**

```bash
git add crates/ui/src/core/widget.rs crates/ui/src/sidebar.rs crates/ui/src/widgets/list.rs
git commit -m "feat: add List/CloseTab/CloseRequested type variants for sidebar close button"
```

---

### Task 2: list.rs 布局常量与辅助计算

**Files:**
- Modify: `crates/ui/src/widgets/list.rs`

- [ ] **Step 1: 在文件顶部（ListItemKind 之前）添加常量**

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

- [ ] **Step 2: 添加 item_pinned_left_offset 辅助函数（private）**

在 `impl VerticalListWidget` 的辅助方法区域（`item_rect` 附近）：

```rust
/// pinned item 左侧锁定图标占用的总宽度（0 表示非 pinned）
fn pinned_left_offset(&self, item: &ListItem, dpi: f32) -> f32 {
    if item.pinned {
        LOCK_ICON_SIZE_LOGICAL * dpi + LOCK_ICON_MARGIN_LOGICAL * dpi
    } else {
        0.0
    }
}

/// 右侧关闭按钮预留宽度（0 表示 pinned，始终预留非 pinned 的空间）
fn close_btn_reserved_width(&self, item: &ListItem, dpi: f32) -> f32 {
    if item.pinned {
        0.0
    } else {
        CLOSE_BTN_SIZE_LOGICAL * dpi + CLOSE_BTN_MARGIN_LOGICAL * dpi
    }
}
```

- [ ] **Step 3: 编译验证**

```bash
cargo check -p edit-plus-ui 2>&1
```

- [ ] **Step 4: Commit**

```bash
git add crates/ui/src/widgets/list.rs
git commit -m "feat: add layout constants and helper fns for sidebar lock icon + close btn"
```

---

### Task 3: 编写 list.rs paint 测试（先写失败的测试）

**Files:**
- Modify: `crates/ui/src/widgets/list.rs` (test module)

- [ ] **Step 1: 添加辅助函数 `pinned_item`**

在 tests 模块中，`fn item` 旁边：

```rust
fn pinned_item(label: &str) -> ListItem {
    ListItem { label: label.into(), kind: ListItemKind::Normal, indicator: ListItemIndicator::None, pinned: true }
}
```

- [ ] **Step 2: 添加测试 pinned_item_paints_lock_icon**

```rust
#[test]
fn pinned_item_paints_lock_icon() {
    let theme = Theme::dark();
    let mut m = NoopMeasure;
    let mut layout = layout_ctx(&theme, &mut m);
    let mut w = VerticalListWidget::new(style());
    w.set_items(vec![pinned_item("pinned.rs")]);
    w.set_rect(Rect::new(0.0, 0.0, 220.0, 100.0), &mut layout);

    let mut list = DrawList::new();
    let mut paint = PaintCtx { global_alpha: 1.0, list: &mut list, theme: &theme, dpi: 1.0,
        offset: (0.0, 0.0),
    };
    w.paint(&mut paint);

    // bg + 2 lock fills + text = 4
    assert_eq!(list.cmds.len(), 4);
}
```

- [ ] **Step 3: 添加测试 non_pinned_item_no_lock_icon**

```rust
#[test]
fn non_pinned_item_no_lock_icon() {
    let theme = Theme::dark();
    let mut m = NoopMeasure;
    let mut layout = layout_ctx(&theme, &mut m);
    let mut w = VerticalListWidget::new(style());
    w.set_items(vec![item("normal.rs")]);
    w.set_rect(Rect::new(0.0, 0.0, 220.0, 100.0), &mut layout);

    let mut list = DrawList::new();
    let mut paint = PaintCtx { global_alpha: 1.0, list: &mut list, theme: &theme, dpi: 1.0,
        offset: (0.0, 0.0),
    };
    w.paint(&mut paint);

    // bg + text = 2（无锁图标填充）
    assert_eq!(list.cmds.len(), 2);
}
```

- [ ] **Step 4: 添加测试 hover_shows_close_button**

```rust
#[test]
fn hover_shows_close_button() {
    let theme = Theme::dark();
    let mut m = NoopMeasure;
    let mut layout = layout_ctx(&theme, &mut m);
    let mut w = VerticalListWidget::new(style());
    w.set_items(vec![item("closeable.rs")]);
    w.set_hovered_index(Some(0));
    w.set_rect(Rect::new(0.0, 0.0, 220.0, 100.0), &mut layout);

    let mut list = DrawList::new();
    let mut paint = PaintCtx { global_alpha: 1.0, list: &mut list, theme: &theme, dpi: 1.0,
        offset: (0.0, 0.0),
    };
    w.paint(&mut paint);

    // bg + hover bg + text + close btn bg + close btn text("✕") = 5
    assert_eq!(list.cmds.len(), 5);
    // 验证有关闭按钮文字
    let has_close_text = list.cmds.iter().any(|c| matches!(c, DrawCmd::Text { content, .. } if content == "✕"));
    assert!(has_close_text, "Expected close button text '✕'");
}
```

- [ ] **Step 5: 运行测试确认失败**

```bash
cargo test -p edit-plus-ui pinned_item_paints_lock_icon non_pinned_item_no_lock_icon hover_shows_close_button 2>&1
```

预期：全部 FAIL（paint 尚未修改）

- [ ] **Step 6: Commit**

```bash
git add crates/ui/src/widgets/list.rs
git commit -m "test: add failing tests for lock icon and close button paint"
```

---

### Task 4: 实现 paint 中的锁定图标和关闭按钮

**Files:**
- Modify: `crates/ui/src/widgets/list.rs`

修改 `paint` 方法中 `ListItemKind::Header | ListItemKind::Normal` 分支。

- [ ] **Step 1: 修改 paint 方法**

替换 `paint` 中 `ListItemKind::Header | ListItemKind::Normal` 分支的整个代码块：

```rust
ListItemKind::Header | ListItemKind::Normal => {
    // hover/active 仅 normal
    if matches!(item.kind, ListItemKind::Normal) {
        let is_active = Some(i) == self.active_index;
        let is_hovered = Some(i) == self.hovered_index;
        if is_active {
            let mut color = self.style.item_active_bg;
            color[3] *= alpha;
            ctx.list.fill_menu_hover(row_rect, color, dpi);
        } else if is_hovered {
            let mut color = self.style.item_hover_bg;
            color[3] *= alpha;
            ctx.list.fill_menu_hover(row_rect, color, dpi);
        }
    }

    let mut text_x = row_rect.x + pad_x;

    // ── 1) 锁定图标（pinned item）──
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
        // 锁扣
        ctx.list.fill(
            Rect::new(lock_x + lock_size * 0.2, lock_y - lock_size * 0.3,
                      lock_size * 0.6, lock_size * 0.3),
            lock_fg,
        );

        text_x += lock_size + lock_margin;
    }

    // ── 2) 关闭按钮区域（始终预留空间）──
    let close_btn_reserved_w = self.close_btn_reserved_width(item, dpi);

    // ── 3) 文字 ──
    let baseline = row_rect.y + row_rect.h * 0.5 + font_size * 0.35;
    let mut fg = self.style.item_fg;
    fg[3] *= alpha;
    let dot_extra = if matches!(item.indicator, ListItemIndicator::Dot) {
        dot_r * 2.0 + 4.0 * dpi
    } else {
        0.0
    };
    let label_max_w = (row_rect.w - pad_x * 2.0 - close_btn_reserved_w - dot_extra
        - (text_x - row_rect.x - pad_x)).max(0.0);
    let label = truncate_title_by_width(&item.label, label_max_w, font_size);
    ctx.list.text(text_x, baseline, font_size, fg, &label);

    // 指示符（文件名后面）— * 表示未保存
    if matches!(item.indicator, ListItemIndicator::Dot) {
        let mut ind = self.style.indicator_color;
        ind[3] *= alpha;
        let label_w = self.label_widths.get(i).copied().unwrap_or(0.0);
        // 限制指示符不超出关闭按钮预留区域
        let max_dot_x = row_rect.x + row_rect.w - pad_x - close_btn_reserved_w - 2.0 * dpi;
        let dot_x = (row_rect.x + pad_x + label_w + 2.0 * dpi).min(max_dot_x);
        ctx.list.text(
            dot_x, baseline, font_size,
            ind, "*",
        );
    }

    // ── 4) 关闭按钮（hover 时绘制）──
    if !item.pinned && Some(i) == self.hovered_index {
        let btn_size = CLOSE_BTN_SIZE_LOGICAL * dpi;
        let btn_x = row_rect.x + row_rect.w - pad_x - btn_size;
        let btn_y = row_rect.y + (row_rect.h - btn_size) * 0.5;

        let mut btn_bg = self.style.item_fg;
        btn_bg[3] = 0.15 * alpha;
        ctx.list.fill(
            Rect::new(btn_x, btn_y, btn_size, btn_size),
            btn_bg,
        );

        let cross_font_size = (self.style.font_size_logical - 2.0) * dpi;
        let mut cross_fg = self.style.item_fg;
        cross_fg[3] *= alpha;
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
```

- [ ] **Step 2: 运行测试确认通过**

```bash
cargo test -p edit-plus-ui pinned_item_paints_lock_icon non_pinned_item_no_lock_icon hover_shows_close_button 2>&1
```

预期：全部 PASS

- [ ] **Step 3: 运行全部现有测试确认无回归**

```bash
cargo test -p edit-plus-ui --lib 2>&1
```

- [ ] **Step 4: Commit**

```bash
git add crates/ui/src/widgets/list.rs
git commit -m "feat: paint lock icon for pinned items and close button on hover"
```

---

### Task 5: 实现 hit_close_btn 和 on_event 修改

**Files:**
- Modify: `crates/ui/src/widgets/list.rs`

- [ ] **Step 1: 添加关闭按钮命中测试（先写测试）**

```rust
#[test]
fn click_close_button_returns_close_requested() {
    let mut w = make_list(
        vec![item("a"), item("b")],
        Rect::new(0.0, 0.0, 220.0, 100.0),
    );
    w.set_hovered_index(Some(0));
    let theme = Theme::dark();
    let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

    // 关闭按钮位于行右侧：row_rect 右边界 - pad_x - btn_size/2 附近
    // dpi=1: row_h=24, pad_y=4, row_rect(0)=(0, 4, 220, 24)
    // btn_x = 220 - 8 - 16 = 196, btn_center = 196 + 8 = 204
    let action = w.on_event(
        &Event::MouseDown { px: 204.0, py: 16.0, button: MouseButton::Left },
        &mut ctx,
    );
    assert!(action.is_some());
    match action.unwrap() {
        WidgetAction::List(ListAction::CloseRequested(0)) => {},
        other => panic!("Expected CloseRequested(0), got {:?}", other),
    }
}

#[test]
fn click_on_row_but_not_close_btn_returns_selected() {
    let mut w = make_list(
        vec![item("a")],
        Rect::new(0.0, 0.0, 220.0, 100.0),
    );
    w.set_hovered_index(Some(0));
    let theme = Theme::dark();
    let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

    // 点在行左侧（文字区域，不是关闭按钮）
    let action = w.on_event(
        &Event::MouseDown { px: 50.0, py: 16.0, button: MouseButton::Left },
        &mut ctx,
    );
    assert!(action.is_some());
    match action.unwrap() {
        WidgetAction::List(ListAction::Selected(0)) => {},
        other => panic!("Expected Selected(0), got {:?}", other),
    }
}

#[test]
fn pinned_item_click_does_not_return_close_requested() {
    let mut w = make_list(
        vec![pinned_item("pinned.rs")],
        Rect::new(0.0, 0.0, 220.0, 100.0),
    );
    w.set_hovered_index(Some(0));
    let theme = Theme::dark();
    let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };

    // 点在关闭按钮可能的位置 — 但对于 pinned item 应该返回 Selected
    let action = w.on_event(
        &Event::MouseDown { px: 204.0, py: 16.0, button: MouseButton::Left },
        &mut ctx,
    );
    assert!(action.is_some());
    match action.unwrap() {
        WidgetAction::List(ListAction::Selected(0)) => {},
        other => panic!("Expected Selected(0) for pinned item, got {:?}", other),
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

```bash
cargo test -p edit-plus-ui click_close_button_returns_close_requested click_on_row_but_not_close_btn_returns_selected pinned_item_click_does_not_return_close_requested 2>&1
```

- [ ] **Step 3: 实现 hit_close_btn 方法**

在 `impl VerticalListWidget` 中（`hit_row` 之后）：

```rust
/// 检测点击是否命中关闭按钮（仅非 pinned 且当前 hover 的行，实时计算矩形）
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
```

- [ ] **Step 4: 修改 on_event 的 MouseDown 分支**

将现有的：

```rust
Event::MouseDown { px, py, button: MouseButton::Left } => {
    self.hit_row(*px, *py, self.scroll_offset, ctx.dpi)
        .map(|_| WidgetAction::Consumed)
}
```

替换为：

```rust
Event::MouseDown { px, py, button: MouseButton::Left } => {
    // 优先检测关闭按钮
    if let Some(idx) = self.hit_close_btn(*px, *py, self.scroll_offset, ctx.dpi) {
        return Some(WidgetAction::List(ListAction::CloseRequested(idx)));
    }
    // 检测行点击
    if let Some(idx) = self.hit_row(*px, *py, self.scroll_offset, ctx.dpi) {
        return Some(WidgetAction::List(ListAction::Selected(idx)));
    }
    None
}
```

- [ ] **Step 5: 运行新测试确认通过**

```bash
cargo test -p edit-plus-ui click_close_button_returns_close_requested click_on_row_but_not_close_btn_returns_selected pinned_item_click_does_not_return_close_requested 2>&1
```

- [ ] **Step 6: 修复因 on_event 返回值变化导致的现有测试失败**

当前几个测试断言 `WidgetAction::Consumed`，需要更新：

`click_in_row_returns_selected_index` 测试：
```rust
#[test]
fn click_in_row_returns_selected_index() {
    let mut w = make_list(
        vec![item("a"), item("b"), item("c")],
        Rect::new(0.0, 0.0, 220.0, 100.0),
    );
    let theme = Theme::dark();
    let mut ctx = EventCtx { cursor_hint: None, theme: &theme, dpi: 1.0 };
    let action = w.on_event(
        &Event::MouseDown { px: 100.0, py: 40.0, button: MouseButton::Left },
        &mut ctx,
    ).unwrap();
    assert_eq!(action, WidgetAction::List(ListAction::Selected(1)));
}
```

`hover_change_emits_action_only_on_change` 测试保持不变（MouseMove 仍返回 `Consumed`）。

- [ ] **Step 7: 运行全部测试确认无回归**

```bash
cargo test -p edit-plus-ui --lib 2>&1
```

- [ ] **Step 8: Commit**

```bash
git add crates/ui/src/widgets/list.rs
git commit -m "feat: implement hit_close_btn and update on_event for close button clicks"
```

---

### Task 6: SidebarWidget 集成

**Files:**
- Modify: `crates/ui/src/widgets/sidebar.rs`

- [ ] **Step 1: 修改 ListItem 构建（set_rect 方法中）**

将 `set_rect` 方法中的 ListItem 构建：

```rust
let items: Vec<ListItem> = self.tabs.iter().map(|t| ListItem {
    label: t.title.clone(),
    kind: ListItemKind::Normal,
    indicator: if t.is_dirty { ListItemIndicator::Dot } else { ListItemIndicator::None },
}).collect();
```

改为：

```rust
let items: Vec<ListItem> = self.tabs.iter().map(|t| ListItem {
    label: t.title.clone(),
    kind: ListItemKind::Normal,
    indicator: if t.is_dirty { ListItemIndicator::Dot } else { ListItemIndicator::None },
    pinned: t.pinned,
}).collect();
```

- [ ] **Step 2: 修改 MouseDown 的 list hit test，委托给 list.on_event()**

在 `on_event` 的 `Event::MouseDown { px, py, button } if *button == MouseButton::Left` 分支中，
找到第 3 步（list hit test）当前代码：

```rust
// 3) Hit test list items; map sorted list index back to workspace index
if let Some(layout) = self.state.current_layout() {
    if let Some(sorted_idx) = self.list.hit_row(px, py, self.list_scroll_offset(), ctx.dpi) {
        let ws_idx = self.tab_index_map.get(sorted_idx).copied().unwrap_or(sorted_idx);
        return Some(WidgetAction::Sidebar(SidebarAction::SwitchTab(ws_idx)));
    }
}
```

替换为：

```rust
// 3) List item hit test & close button — delegate to list.on_event()
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
```

- [ ] **Step 3: 编译验证**

```bash
cargo check -p edit-plus-ui 2>&1
```

- [ ] **Step 4: 运行 sidebar widget 测试**

```bash
cargo test -p edit-plus-ui --lib sidebar 2>&1
```

- [ ] **Step 5: Commit**

```bash
git add crates/ui/src/widgets/sidebar.rs
git commit -m "feat: integrate close button and pinned state into SidebarWidget"
```

---

### Task 7: events.rs 处理 CloseTab

**Files:**
- Modify: `crates/app/src/events.rs`

- [ ] **Step 1: 在 translate_sidebar_action 中添加 CloseTab 分支**

在 `S::SwitchTab(idx)` 旁边添加：

```rust
fn translate_sidebar_action(
    sa: &ui::sidebar::SidebarAction,
    actions: &mut Vec<AppAction>,
) {
    use ui::sidebar::SidebarAction as S;
    match sa {
        S::SwitchTab(idx) => actions.push(AppAction::SwitchTab(*idx)),
        S::CloseTab(idx) => actions.push(AppAction::CloseTab(*idx)),
        // ...existing arms...
    }
}
```

- [ ] **Step 2: 编译验证**

```bash
cargo check -p edit-plus-app 2>&1
```

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/events.rs
git commit -m "feat: translate SidebarAction::CloseTab to AppAction::CloseTab"
```

---

### Task 8: 添加 sidebar widget 集成测试

**Files:**
- Modify: `crates/ui/src/widgets/sidebar.rs` (test module)

- [ ] **Step 1: 添加测试 pinned_tab_has_pinned_field_in_list_item**

```rust
#[test]
fn pinned_tab_has_pinned_field_in_list_item() {
    let cfg = SidebarConfig::new_default(1.0);
    let mut w = SidebarWidget::new(cfg, 1.0);
    w.set_visibility(Visibility::Pinned);
    let tabs = vec![
        make_tab("a.rs"),
        TabInfo {
            title: "pinned.rs".into(),
            file_path: None,
            is_dirty: false,
            pinned: true,
            language: "rust".into(),
        },
    ];
    w.set_input(tabs, Some(0), (0.0, 0.0), 1200.0, 800.0);

    let t = test_theme();
    let mut m = NoopMeasure;
    let mut lc = LayoutCtx { measure: &mut m, theme: &t, dpi: 1.0 };
    w.set_rect(Rect::new(0.0, 0.0, 220.0, 800.0), &mut lc);

    let items = w.list.items();
    assert_eq!(items.len(), 2);
    assert!(!items[0].pinned, "first tab should not be pinned");
    assert!(items[1].pinned, "second tab should be pinned");
}
```

- [ ] **Step 2: 运行测试**

```bash
cargo test -p edit-plus-ui pinned_tab_has_pinned_field_in_list_item 2>&1
```

预期：PASS

- [ ] **Step 3: Commit**

```bash
git add crates/ui/src/widgets/sidebar.rs
git commit -m "test: verify pinned tab maps to pinned ListItem"
```

---

### Task 9: 添加关闭按钮空间预留测试

**Files:**
- Modify: `crates/ui/src/widgets/list.rs` (test module)

- [ ] **Step 1: 添加测试 close_button_space_reserved_when_not_hovered**

```rust
#[test]
fn close_button_space_reserved_when_not_hovered() {
    let theme = Theme::dark();
    let mut m = NoopMeasure;
    let mut layout = layout_ctx(&theme, &mut m);
    let mut w = VerticalListWidget::new(style());
    let label = "a_very_long_filename_that_should_be_truncated.rs";
    w.set_items(vec![item(label)]);
    // 窄宽度 — 文字必须被截断才能放得下（包括关闭按钮预留空间）
    w.set_rect(Rect::new(0.0, 0.0, 180.0, 100.0), &mut layout);
    // 未 hover
    w.set_hovered_index(None);

    let mut list = DrawList::new();
    let mut paint = PaintCtx { global_alpha: 1.0, list: &mut list, theme: &theme, dpi: 1.0,
        offset: (0.0, 0.0),
    };
    w.paint(&mut paint);

    let text_cmd = list.cmds.iter().find_map(|c| match c {
        DrawCmd::Text { content, .. } if content != "✕" && content != "*" => Some(content),
        _ => None,
    }).unwrap();
    assert!(text_cmd.contains('…'), "Label should be truncated with ellipsis, got: {text_cmd}");
    // 加上关闭按钮预留空间后，即使未 hover 也截断
    assert!(text_cmd.len() < label.len());
}
```

- [ ] **Step 2: 运行测试**

```bash
cargo test -p edit-plus-ui close_button_space_reserved_when_not_hovered 2>&1
```

- [ ] **Step 3: Commit**

```bash
git add crates/ui/src/widgets/list.rs
git commit -m "test: verify close button space reserved even when not hovered"
```

---

### Task 10: 全量构建和测试

- [ ] **Step 1: 检查编译**

```bash
cargo check -p edit-plus-app 2>&1
```

- [ ] **Step 2: 运行全部单元测试**

```bash
cargo test -p edit-plus-ui --lib 2>&1
cargo test -p edit-plus-app --lib 2>&1
```

- [ ] **Step 3: 检查是否有遗漏的 exhaustive match**

```bash
cargo clippy -p edit-plus-ui 2>&1 | head -30
cargo clippy -p edit-plus-app 2>&1 | head -30
```

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: final verification — all tests pass, no clippy warnings"
```

---

### 任务依赖图

```
Task 1 (类型定义)
  └─> Task 2 (常量/辅助函数)
        └─> Task 3 (paint 测试: 先写)
              └─> Task 4 (paint 实现: 使测试通过)
                    └─> Task 5 (hit_close_btn + on_event 测试与实现)
                          └─> Task 6 (SidebarWidget 集成)
                                ├─> Task 7 (events.rs CloseTab 处理)
                                ├─> Task 8 (sidebar 集成测试)
                                └─> Task 9 (预留空间测试)
                                      └─> Task 10 (全量验证)
```

Task 7、8、9 不互相依赖，可并行。
