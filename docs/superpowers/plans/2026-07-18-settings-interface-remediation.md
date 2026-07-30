# 设置界面交互与视觉修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让侧栏设置直达模态设置页，并使设置控件在任意主题下可读、符合 macOS 设置的紧凑布局且可输入。

**Architecture:** 保持 `ui` 层只暴露纯 WidgetAction，并在 `app` 层重定向侧栏设置动作和模态键盘输入。设置页从 `SettingsTheme` 派生按钮样式；开关把父级矩形作为命中区域、把固定尺寸轨道右对齐绘制。

**Tech Stack:** Rust、winit、textora-ui widget 系统、Cargo 测试。

## Global Constraints

- 产品名为 textora，Markdown 包名为 `textora-markdown`。
- `ui` 不得依赖或访问 `app` 状态；跨层操作使用现有 `WidgetAction` 和纯数据输入。
- 所有新行为先写失败测试，再写最小实现；不使用 `.unwrap()`。
- 分类导航：28 逻辑像素高、2 逻辑像素间距、水平内缩 8 逻辑像素；仅选中项高亮。
- Switch 轨道：36×20 逻辑像素，垂直居中、右对齐控制列。

---

## 文件与职责

| 文件 | 职责 |
| --- | --- |
| `crates/app/src/app_dispatch.rs` | 将侧栏设置动作直接映射为模态设置页。 |
| `crates/app/src/app_lifecycle.rs` | 在编辑器分发前优先路由活动模态页的键盘和 IME 动作。 |
| `crates/ui/src/widgets/settings_view/widget.rs` | 主题化参数按钮，布局 macOS 风格分类导航，保持文本框焦点。 |
| `crates/ui/src/widgets/switch.rs` | 计算固定尺寸、右对齐的开关轨道。 |

### Task 1: 侧栏设置入口改为模态页

**Files:**
- Modify: `crates/app/src/app_dispatch.rs:190-205, 730-780`

**Interfaces:**
- Consumes: `AppAction::OpenSidebarSettingsMenu`。
- Produces: `App::open_settings_from_sidebar() -> AppEffect`；`UiShell::active_overlay_is_modal() == true`。

- [ ] **Step 1: 写失败测试**

在 `app_dispatch.rs` 的测试模块添加：

```rust
#[test]
fn sidebar_settings_action_opens_modal_settings_view() {
    let mut app = App::new(None);

    let effect = app.open_settings_from_sidebar();

    assert!(effect.redraw);
    assert_eq!(app.ui_shell.overlays_count(), 1);
    assert!(app.ui_shell.active_overlay_is_modal());
}
```

- [ ] **Step 2: 验证测试失败**

运行：`cargo test -p textora-app sidebar_settings_action_opens_modal_settings_view --lib`

预期：失败，因为当前动作仅构造 sidebar popup，overlay 数量为 0。

- [ ] **Step 3: 写最小实现**

新增只封装侧栏入口的私有方法，并将 `AppAction::OpenSidebarSettingsMenu` 的分支委托给它：

```rust
fn open_settings_from_sidebar(&mut self) -> AppEffect {
    self.open_settings_overlay()
}

// reduce_action 的 match 分支：
AppAction::OpenSidebarSettingsMenu => self.open_settings_from_sidebar(),
```

不修改 `SidebarAction` 协议；也不调用 `dispatch_chrome_action(ChromeDispatchAction::OpenSidebarSettingsMenu)`。

- [ ] **Step 4: 验证测试通过**

运行：`cargo test -p textora-app sidebar_settings_action_opens_modal_settings_view --lib`

预期：1 个测试通过。

- [ ] **Step 5: 提交该独立行为**

```bash
git add crates/app/src/app_dispatch.rs
git commit -m "fix(app): open settings overlay from sidebar"
```

### Task 2: 设置模态页接收键盘与 IME 输入

**Files:**
- Modify: `crates/app/src/app_lifecycle.rs:242-355`
- Test: `crates/app/src/app_lifecycle.rs` 的现有测试模块

**Interfaces:**
- Consumes: `UiShell::forward_key` 和 `UiShell::forward_ime` 返回的 `WidgetAction::Settings(SettingsViewAction)`。
- Produces: `AppAction::Settings` 分发；未打开模态页时保持既有编辑器和搜索框路由。

- [ ] **Step 1: 写失败测试**

提取生命周期内部用于处理 overlay 返回值的私有函数，并先测试设置动作会被译为应用动作：

```rust
#[test]
fn modal_settings_action_is_dispatched_instead_of_only_redrawing() {
    let action = WidgetAction::Settings(SettingsViewAction::SetFontFamily("Menlo".into()));

    assert_eq!(modal_widget_action(&action), Some(AppAction::Settings(
        SettingsViewAction::SetFontFamily("Menlo".into()),
    )));
}
```

- [ ] **Step 2: 验证测试失败**

运行：`cargo test -p textora-app modal_settings_action_is_dispatched_instead_of_only_redrawing --lib`

预期：失败，因为 `modal_widget_action` 尚不存在。

- [ ] **Step 3: 写最小实现**

实现只映射设置动作的帮助函数：

```rust
fn modal_widget_action(action: &WidgetAction) -> Option<AppAction> {
    match action {
        WidgetAction::Settings(settings_action) => Some(AppAction::Settings(settings_action.clone())),
        _ => None,
    }
}
```

在 `KeyboardInput` 分支、搜索框专用分支之前调用 `forward_key`；若返回设置动作则 `dispatch`，若返回其他模态动作则重绘并停止继续向编辑器分发。`Ime::Commit` 的 `action.is_some()` 分支同样先分发该映射，只有未映射动作才仅重绘。

- [ ] **Step 4: 验证测试通过**

运行：`cargo test -p textora-app modal_settings_action_is_dispatched_instead_of_only_redrawing --lib`

预期：1 个测试通过。

- [ ] **Step 5: 提交该独立行为**

```bash
git add crates/app/src/app_lifecycle.rs
git commit -m "fix(app): route modal settings text input"
```

### Task 3: 主题化设置按钮与 macOS 风格导航

**Files:**
- Modify: `crates/ui/src/widgets/settings_view/widget.rs:19-58, 140-170, 570-610, 731-746, 780-900`

**Interfaces:**
- Consumes: `SettingsTheme { accent, text_primary, control_border, control_surface }`。
- Produces: `parameter_button_style(SettingsTheme) -> ButtonStyle` 和 `category_button_style(SettingsTheme) -> ButtonStyle`。

- [ ] **Step 1: 写失败测试**

在该文件测试模块添加：

```rust
#[test]
fn parameter_buttons_use_theme_primary_text_when_unselected() {
    let settings = crate::theme::test_theme().settings_theme();

    assert_eq!(parameter_button_style(settings).foreground, settings.text_primary);
}

#[test]
fn category_navigation_uses_compact_mac_like_geometry() {
    let mut view = settings_fixture(SettingsCategory::Appearance);
    let theme = crate::theme::test_theme();
    let mut measure = NoopMeasure;
    let mut layout = LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    view.set_rect(Rect::new(0.0, 0.0, 600.0, 400.0), &mut layout);

    assert_eq!(view.category_rects[0].x, 8.0);
    assert_eq!(view.category_rects[0].h, 28.0);
    assert_eq!(view.category_rects[1].y - view.category_rects[0].bottom(), 2.0);
    assert_eq!(category_button_style(crate::theme::test_theme().settings_theme()).border[3], 0.0);
}
```

- [ ] **Step 2: 验证测试失败**

运行：`cargo test -p textora-ui "parameter_buttons_use_theme_primary_text_when_unselected|category_navigation_uses_compact_mac_like_geometry" --lib`

预期：失败，因为样式函数和紧凑几何尚不存在。

- [ ] **Step 3: 写最小实现**

移除固定的 `SETTINGS_BUTTON_*` RGBA 常量。用 `SettingsTheme` 建立参数和分类两种 `ButtonStyle`；分类样式的 `background`、`border`、`hover_background` 都为透明，`selected_background` 为由 `accent` 派生的低饱和色。设置分类矩形为：

```rust
let category_rect = Rect::new(
    8.0 * ctx.dpi,
    category_y,
    (sidebar_width - 16.0 * ctx.dpi).max(0.0),
    28.0 * ctx.dpi,
);
category_y += category_rect.h + 2.0 * ctx.dpi;
```

在 `set_rect` 中以当前 `ctx.theme.settings_theme()` 更新已有分类按钮；在创建主题、视图模式和重试按钮时使用参数按钮样式。确保主题切换后 `set_input` 重建的控件使用新主题。

- [ ] **Step 4: 验证测试通过**

运行：`cargo test -p textora-ui "parameter_buttons_use_theme_primary_text_when_unselected|category_navigation_uses_compact_mac_like_geometry" --lib`

预期：2 个测试通过。

- [ ] **Step 5: 提交该独立行为**

```bash
git add crates/ui/src/widgets/settings_view/widget.rs
git commit -m "fix(ui): align settings controls with theme"
```

### Task 4: 固定尺寸并右对齐 Switch

**Files:**
- Modify: `crates/ui/src/widgets/switch.rs:9-66, 230-360`

**Interfaces:**
- Consumes: 控制列分配给 `Switch::set_rect` 的矩形和 `PaintCtx::dpi`。
- Produces: `track_rect()` 返回 36×20 逻辑像素、右对齐且居中的物理矩形。

- [ ] **Step 1: 写失败测试**

在 `switch.rs` 测试模块添加：

```rust
#[test]
fn switch_track_keeps_mac_size_and_trailing_alignment_in_wide_control_column() {
    let theme = crate::theme::test_theme();
    let mut measure = NoopMeasure;
    let mut layout = LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    let mut switch = Switch::new(WidgetId(99), false);
    switch.set_rect(Rect::new(0.0, 0.0, 220.0, 32.0), &mut layout);

    assert_eq!(switch.track_rect(), Rect::new(184.0, 6.0, 36.0, 20.0));
}
```

- [ ] **Step 2: 验证测试失败**

运行：`cargo test -p textora-ui switch_track_keeps_mac_size_and_trailing_alignment_in_wide_control_column --lib`

预期：失败，因为当前轨道等于完整 220×32 控制列。

- [ ] **Step 3: 写最小实现**

使用语义常量并改写 `track_rect`：

```rust
let width = (SWITCH_WIDTH_LOGICAL * dpi).min(self.rect.w);
let height = (SWITCH_HEIGHT_LOGICAL * dpi).min(self.rect.h);
Rect::new(
    self.rect.right() - width,
    self.rect.y + (self.rect.h - height) * 0.5,
    width,
    height,
)
```

将 `thumb_rect` 的轨道来源改为 `self.track_rect_with_dpi(dpi)`，使缩放尺寸与轨道一致。鼠标命中继续使用完整控制列矩形。

- [ ] **Step 4: 验证测试通过**

运行：`cargo test -p textora-ui switch_track_keeps_mac_size_and_trailing_alignment_in_wide_control_column --lib`

预期：1 个测试通过。

- [ ] **Step 5: 提交该独立行为**

```bash
git add crates/ui/src/widgets/switch.rs
git commit -m "fix(ui): right align compact settings switches"
```

### Task 5: 全量验证

**Files:**
- Modify: 仅在格式化产生必要变更时修改上述四个源文件。

**Interfaces:**
- Consumes: Task 1 至 Task 4 的已提交修复。
- Produces: 通过格式、定向测试、应用编译与项目验证脚本的工作树。

- [ ] **Step 1: 格式化并检查差异**

运行：`cargo fmt --check`

若失败，运行：`cargo fmt`，然后运行：`git diff --check`。

- [ ] **Step 2: 运行定向测试**

运行：

```bash
cargo test -p textora-app sidebar_settings_action_opens_modal_settings_view --lib
cargo test -p textora-app modal_settings_action_is_dispatched_instead_of_only_redrawing --lib
cargo test -p textora-ui --lib settings_view
cargo test -p textora-ui switch_track_keeps_mac_size_and_trailing_alignment_in_wide_control_column --lib
```

预期：所有命令退出码为 0。

- [ ] **Step 3: 编译与项目验证**

运行：

```bash
cargo check -p textora-app
./scripts/verify.sh
```

预期：所有命令退出码为 0。

- [ ] **Step 4: 提交验证后的格式化改动（如有）**

```bash
git add crates/app/src/app_dispatch.rs crates/app/src/app_lifecycle.rs crates/ui/src/widgets/settings_view/widget.rs crates/ui/src/widgets/switch.rs
git commit -m "style: format settings interface fixes"
```
