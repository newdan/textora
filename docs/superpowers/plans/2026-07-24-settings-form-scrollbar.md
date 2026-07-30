# Settings Form Scrollbar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 提高设置弹窗默认高度，并让所有溢出的设置表单使用现有纵向 `ScrollbarWidget`。

**Architecture:** `App` 仍只负责设置弹窗的首选尺寸；`FormView` 继续独占表单的像素滚动偏移，并组合现有 `ScrollbarWidget` 负责可视化和指针交互。滚轮、轨道翻页和滑块拖动最终都更新同一个 `FormView::scroll_offset`，UI 层不引入 app 状态或同步业务类型。

**Tech Stack:** Rust 2024、textora `ui` widget 系统、现有 `ScrollbarWidget`、Cargo 单元测试。

## Global Constraints

- 设置弹窗默认高度为 560 逻辑像素，继续受窗口高度 90% 上限约束。
- 必须复用 `ui::scrollbar::ScrollbarWidget`，不得创建新的滚动条控件或主题。
- 滚动条覆盖在表单右侧，不压缩表单分区和控件宽度。
- 内容未溢出时不绘制、不命中滚动条；内容溢出时支持滚轮、拖动和轨道翻页。
- 滚动条的视口、内容总量和当前位置必须使用同一高精度坐标单位，不能因整数化内容高度改变最大滚动范围。
- `FormView` 是滚动状态的唯一所有者；滚动条只接收 `ScrollbarInput` 并输出 `ScrollbarAction`。
- 不修改 Syncthing 数据、设置输入结构或 app 层业务状态。
- 不新增依赖。

---

### Task 1: 提高设置弹窗首选高度

**Files:**
- Modify: `crates/app/src/settings_overlay.rs:5-20`
- Test: `crates/app/src/settings_overlay.rs:132-143`

**Interfaces:**
- Consumes: `ui::OverlayLayout::Centered` 现有尺寸解析。
- Produces: `settings_overlay_layout() -> ui::OverlayLayout`，首选尺寸为 `720 × 560` 逻辑像素。

- [ ] **Step 1: 修改现有测试，先表达 560 高度**

将测试改为：

```rust
#[test]
fn settings_overlay_uses_expanded_preferred_height() {
    assert_eq!(
        settings_overlay_layout().resolve(ui::Rect::new(0.0, 0.0, 1200.0, 800.0), 1.0),
        ui::Rect::new(240.0, 120.0, 720.0, 560.0),
    );
}
```

- [ ] **Step 2: 运行测试，确认因旧高度失败**

Run:

```bash
cargo test -p textora-app --lib -- settings_overlay_uses_expanded_preferred_height
```

Expected: FAIL；实际矩形高度仍为 `440.0`，纵坐标仍为 `180.0`。

- [ ] **Step 3: 做最小生产修改**

修改常量：

```rust
const SETTINGS_OVERLAY_PREFERRED_HEIGHT_LOGICAL: f32 = 560.0;
```

保留 `SETTINGS_OVERLAY_MAX_HEIGHT_RATIO = 0.90` 和其他布局参数不变。

- [ ] **Step 4: 运行目标测试和 settings overlay 测试**

Run:

```bash
cargo test -p textora-app --lib -- settings_overlay
```

Expected: 所有匹配测试 PASS，0 failed。

- [ ] **Step 5: 提交弹窗高度修改**

```bash
git add crates/app/src/settings_overlay.rs
git commit -m "fix(settings): increase overlay height"
```

---

### Task 2: 在 FormView 中组合现有滚动条

**Files:**
- Modify: `crates/ui/src/widgets/form/view.rs:1-390`
- Test: `crates/ui/src/widgets/form/view.rs:390-980`

**Interfaces:**
- Consumes:
  - `crate::widgets::scrollbar::{SCROLLBAR_RESERVE_PX, ScrollbarAction, ScrollbarInput, ScrollbarWidget}`
  - `ScrollbarWidget::set_input(ScrollbarInput)`
  - `WidgetAction::Scrollbar(ScrollbarAction)`
- Produces:
  - `FormView` 内部纵向滚动条组合。
  - `FormView::scroll_offset() -> f32` 继续返回唯一滚动位置。
  - `FormView::is_capturing()` 同时反映表单子控件和滚动条拖动。

- [ ] **Step 1: 添加短内容隐藏和长内容输入同步的失败测试**

在测试模块加入：

```rust
#[test]
fn form_view_hides_scrollbar_when_content_fits() {
    let view = laid_out_form_view(content_height(200.0), viewport_height(300.0));

    assert_eq!(view.scrollbar_rect, Rect::ZERO);
    assert_eq!(view.scroll_offset(), 0.0);
}

#[test]
fn form_view_configures_scrollbar_from_pixel_scroll_state() {
    let view = laid_out_form_view(content_height(900.0), viewport_height(300.0));

    assert_eq!(view.scrollbar_rect, Rect::new(706.0, 0.0, 14.0, 300.0));
    assert_eq!(
        view.scrollbar.input,
        ScrollbarInput {
            viewport_height_px: 300_000.0,
            total_display_rows: 900_000,
            scroll_top_rows: 0.0,
        },
    );
}
```

更新 `laid_out_form_view`，在覆盖测试内容高度后重新计算滚动条布局：

```rust
view.content_height = content_height;
view.layout_scrollbar(&mut ctx);
```

- [ ] **Step 2: 运行测试，确认缺少组合字段和布局方法**

Run:

```bash
cargo test -p textora-ui --lib -- form_view_hides_scrollbar_when_content_fits
cargo test -p textora-ui --lib -- form_view_configures_scrollbar_from_pixel_scroll_state
```

Expected: 编译 FAIL；`FormView` 尚无 `scrollbar`、`scrollbar_rect` 和 `layout_scrollbar`。

- [ ] **Step 3: 添加滚动条字段、输入映射和布局**

添加导入：

```rust
use crate::widgets::scrollbar::{
    SCROLLBAR_RESERVE_PX, ScrollbarAction, ScrollbarInput, ScrollbarWidget,
};
```

在 `FormView` 增加：

```rust
scrollbar: ScrollbarWidget,
scrollbar_rect: Rect,
```

增加统一的高精度坐标比例，传给既有 `ScrollbarWidget` 的三个数值都使用该单位：

```rust
const FORM_SCROLLBAR_COORDINATE_SCALE: f64 = 1_000.0;
```

在 `FormView::new` 初始化：

```rust
scrollbar: ScrollbarWidget::vertical(),
scrollbar_rect: Rect::ZERO,
```

加入以下方法：

```rust
fn has_overflow(&self) -> bool {
    self.content_height > self.rect.h
}

fn scrollbar_input(&self) -> ScrollbarInput {
    ScrollbarInput {
        viewport_height_px: self.rect.h as f64 * FORM_SCROLLBAR_COORDINATE_SCALE,
        total_display_rows: (self.content_height.max(0.0) as f64
            * FORM_SCROLLBAR_COORDINATE_SCALE)
            .ceil() as usize,
        scroll_top_rows: self.scroll_offset as f64 * FORM_SCROLLBAR_COORDINATE_SCALE,
    }
}

fn sync_scrollbar_input(&mut self) {
    self.scrollbar.set_input(self.scrollbar_input());
}

fn layout_scrollbar(&mut self, ctx: &mut LayoutCtx) {
    self.sync_scrollbar_input();
    if !self.has_overflow() || self.rect.w <= 0.0 || self.rect.h <= 0.0 {
        self.scrollbar_rect = Rect::ZERO;
        self.scrollbar.set_rect(Rect::ZERO, ctx);
        return;
    }

    let scrollbar_width = (SCROLLBAR_RESERVE_PX * ctx.dpi).min(self.rect.w);
    self.scrollbar_rect =
        Rect::new(self.rect.w - scrollbar_width, 0.0, scrollbar_width, self.rect.h);
    self.scrollbar.set_rect(
        Rect::new(0.0, 0.0, self.scrollbar_rect.w, self.scrollbar_rect.h),
        ctx,
    );
}
```

在 `layout_sections` 的空分区分支和正常分支完成内容高度、偏移限制后调用：

```rust
self.layout_scrollbar(ctx);
```

- [ ] **Step 4: 运行两个布局测试，确认通过**

Run:

```bash
cargo test -p textora-ui --lib -- form_view_hides_scrollbar_when_content_fits
cargo test -p textora-ui --lib -- form_view_configures_scrollbar_from_pixel_scroll_state
```

Expected: 两个测试 PASS。

- [ ] **Step 5: 添加滚轮后输入同步的失败测试**

加入：

```rust
#[test]
fn form_view_wheel_keeps_scrollbar_position_in_sync() {
    let mut view = laid_out_form_view(content_height(900.0), viewport_height(300.0));

    assert_eq!(wheel(&mut view, -120.0), Some(WidgetAction::Consumed));

    assert_eq!(view.scroll_offset(), 120.0);
    assert_eq!(view.scrollbar.input.scroll_top_rows, 120_000.0);
}

#[test]
fn form_view_scrollbar_preserves_fractional_scroll_range() {
    let mut view = laid_out_form_view(content_height(300.1), viewport_height(300.0));

    assert_eq!(view.scrollbar.input.viewport_height_px, 300_000.0);
    assert_eq!(view.scrollbar.input.total_display_rows, 300_100);

    view.apply_scrollbar_action(ScrollbarAction::DragTo(100.0));

    assert!((view.scroll_offset() - 0.1).abs() < 0.001);
}
```

- [ ] **Step 6: 运行测试，确认滚动条仍保留旧位置**

Run:

```bash
cargo test -p textora-ui --lib -- form_view_wheel_keeps_scrollbar_position_in_sync
```

Expected: FAIL；`scroll_offset()` 为 `120.0`，但 `scrollbar.input.scroll_top_rows` 仍为 `0.0`。

- [ ] **Step 7: 统一所有偏移写入并同步滚动条**

新增：

```rust
fn set_scroll_offset(&mut self, scroll_offset: f32) -> bool {
    let previous_offset = self.scroll_offset;
    self.scroll_offset = scroll_offset.clamp(0.0, self.max_scroll_offset());
    self.sync_scrollbar_input();
    self.scroll_offset != previous_offset
}
```

将现有方法改为：

```rust
pub fn reset_scroll(&mut self) {
    let _ = self.set_scroll_offset(0.0);
}

fn clamp_scroll_offset(&mut self) {
    let _ = self.set_scroll_offset(self.scroll_offset);
}

fn scroll_by(&mut self, delta: f32) -> bool {
    self.set_scroll_offset(self.scroll_offset + delta)
}
```

在 `replace_sections_preserving_state` 恢复旧位置时使用：

```rust
let _ = self.set_scroll_offset(previous_scroll);
```

- [ ] **Step 8: 运行滚轮同步与现有表单滚动测试**

Run:

```bash
cargo test -p textora-ui --lib -- form_view_wheel_keeps_scrollbar_position_in_sync
cargo test -p textora-ui --lib -- form_view_clips_sections_and_clamps_scroll
cargo test -p textora-ui --lib -- replacing_sections_preserves_scroll_and_focus
```

Expected: 三个测试 PASS。

- [ ] **Step 9: 添加轨道翻页、拖动和释放捕获的失败测试**

加入事件辅助函数：

```rust
fn pointer_event(view: &mut FormView, event: Event) -> Option<WidgetAction> {
    let theme = crate::theme::test_theme();
    let mut ctx = event_ctx(&theme);
    view.on_event(&event, &mut ctx)
}
```

加入测试：

```rust
#[test]
fn form_view_scrollbar_pages_and_drags_the_same_scroll_offset() {
    let mut view = laid_out_form_view(content_height(900.0), viewport_height(300.0));

    assert_eq!(
        pointer_event(
            &mut view,
            Event::MouseDown { px: 715.0, py: 200.0, button: MouseButton::Left },
        ),
        Some(WidgetAction::Consumed),
    );
    assert_eq!(view.scroll_offset(), 300.0);

    assert_eq!(
        pointer_event(
            &mut view,
            Event::MouseDown { px: 715.0, py: 150.0, button: MouseButton::Left },
        ),
        Some(WidgetAction::Consumed),
    );
    assert!(view.is_capturing());

    assert_eq!(
        pointer_event(&mut view, Event::MouseMove { px: 715.0, py: 250.0 }),
        Some(WidgetAction::Consumed),
    );
    assert_eq!(view.scroll_offset(), 600.0);

    assert_eq!(
        pointer_event(
            &mut view,
            Event::MouseUp { px: 760.0, py: 250.0, button: MouseButton::Left },
        ),
        Some(WidgetAction::Consumed),
    );
    assert!(!view.is_capturing());
    assert_eq!(view.scrollbar.input.scroll_top_rows, 600_000.0);
}
```

这里初始滑块高度为 100 像素；第一次点击 `y = 200` 触发向下翻页到 300。此时滑块位于
`y = 100..200`，第二次点击 `y = 150` 开始拖动，再移动 100 像素到达最大偏移 600。

- [ ] **Step 10: 运行测试，确认事件尚未连接到滚动条**

Run:

```bash
cargo test -p textora-ui --lib -- form_view_scrollbar_pages_and_drags_the_same_scroll_offset
```

Expected: FAIL；右侧点击仍进入表单分区，或没有更新 `scroll_offset`。

- [ ] **Step 11: 映射滚动条事件和动作**

加入：

```rust
fn scrollbar_event<'a>(&self, event: &'a Event) -> Cow<'a, Event> {
    crate::core::dock::Dock::to_local(
        event,
        self.scrollbar_rect.x,
        self.scrollbar_rect.y,
    )
}

fn apply_scrollbar_action(&mut self, action: ScrollbarAction) {
    match action {
        ScrollbarAction::DragTo(scroll_offset) => {
            let _ = self.set_scroll_offset(
                (scroll_offset / FORM_SCROLLBAR_COORDINATE_SCALE) as f32,
            );
        }
        ScrollbarAction::PageUp => {
            let _ = self.scroll_by(-self.rect.h);
        }
        ScrollbarAction::PageDown => {
            let _ = self.scroll_by(self.rect.h);
        }
        ScrollbarAction::StartDrag
        | ScrollbarAction::EndDrag
        | ScrollbarAction::HoverChanged(_) => {}
    }
}

fn dispatch_scrollbar_event(
    &mut self,
    event: &Event,
    ctx: &mut EventCtx,
) -> Option<WidgetAction> {
    let local_event = self.scrollbar_event(event);
    let action = self.scrollbar.on_event(local_event.as_ref(), ctx)?;
    let WidgetAction::Scrollbar(scrollbar_action) = action else {
        return Some(WidgetAction::Consumed);
    };
    self.apply_scrollbar_action(scrollbar_action);
    Some(WidgetAction::Consumed)
}
```

在 `on_event` 最前面处理拖动捕获：

```rust
if self.scrollbar.is_dragging()
    && matches!(event, Event::MouseMove { .. } | Event::MouseUp { .. })
{
    return self.dispatch_scrollbar_event(event, ctx);
}
```

在 `MouseDown` 分支最前面处理右侧滚动条：

```rust
if self.scrollbar_rect.contains(*px, *py) {
    return self.dispatch_scrollbar_event(event, ctx);
}
```

在 `MouseMove` 分支先把事件发送给滚动条以维护 hover；指针位于滚动条时消费事件，
指针离开时继续原有分区 hover 路由：

```rust
let scrollbar_action = self.dispatch_scrollbar_event(event, ctx);
if self.scrollbar_rect.contains(*px, *py) {
    return scrollbar_action.or(Some(WidgetAction::Consumed));
}
```

在 `MouseUp` 分支保留原有表单指针释放逻辑；滚动条拖动已由最前面的捕获分支处理。

将 `is_capturing` 实现改为：

```rust
fn is_capturing(&self) -> bool {
    self.scrollbar.is_dragging() || self.capturing_section_index().is_some()
}
```

- [ ] **Step 12: 运行滚动条交互测试，确认通过**

Run:

```bash
cargo test -p textora-ui --lib -- form_view_scrollbar_pages_and_drags_the_same_scroll_offset
```

Expected: PASS。

- [ ] **Step 13: 添加滚动条绘制回归断言并观察失败**

修改现有裁剪测试，避免假设 `PopClip` 是最后一个绘制命令，并断言裁剪之后仍有滚动条绘制：

```rust
let pop_clip_index = draw
    .cmds
    .iter()
    .rposition(|command| matches!(command, DrawCmd::PopClip))
    .expect("form content should close its clip");
assert!(matches!(draw.cmds.first(), Some(DrawCmd::PushClip(_))));
assert!(
    draw.cmds.len() > pop_clip_index + 1,
    "overflowing form should paint its scrollbar after clipped content",
);
```

- [ ] **Step 14: 运行测试，确认滚动条尚未绘制**

Run:

```bash
cargo test -p textora-ui --lib -- form_view_clips_sections_and_clamps_scroll
```

Expected: FAIL；`PopClip` 后没有滚动条绘制命令。

- [ ] **Step 15: 在裁剪内容之后绘制滚动条**

在 `FormView::paint` 的现有裁剪块完成后加入：

```rust
if self.scrollbar_rect.w > 0.0 && self.scrollbar_rect.h > 0.0 {
    ctx.list.offset = (
        saved_offset.0 + self.scrollbar_rect.x,
        saved_offset.1 + self.scrollbar_rect.y,
    );
    self.scrollbar.paint(ctx);
}
ctx.list.offset = saved_offset;
```

- [ ] **Step 16: 运行 FormView 全部测试**

Run:

```bash
cargo test -p textora-ui --lib -- widgets::form::view::tests
```

Expected: 所有 `FormView` 测试 PASS，0 failed。

- [ ] **Step 17: 格式化并运行相关 UI 回归测试**

Run:

```bash
cargo fmt --all -- --check
cargo test -p textora-ui --lib -- widgets::form
cargo test -p textora-ui --lib -- widgets::settings_view
```

Expected: 格式检查通过；表单和设置视图测试全部 PASS。

- [ ] **Step 18: 提交 FormView 滚动条集成**

```bash
git add crates/ui/src/widgets/form/view.rs
git commit -m "feat(ui): integrate scrollbar into form view"
```

---

### Task 3: 全量验证

**Files:**
- Verify only; no production file changes expected.

**Interfaces:**
- Consumes: Task 1 和 Task 2 的两个独立提交。
- Produces: 编译与项目验证证据。

- [ ] **Step 1: 运行格式化检查**

Run:

```bash
cargo fmt --all -- --check
```

Expected: exit 0，无格式差异。

- [ ] **Step 2: 运行两个相关 crate 的完整测试**

Run:

```bash
cargo test -p textora-ui --lib
cargo test -p textora-app --lib
```

Expected: 两个命令均 exit 0，0 failed。

- [ ] **Step 3: 运行编译检查**

Run:

```bash
cargo check -p textora-ui
cargo check -p textora-app
```

Expected: 两个命令均 exit 0，无编译错误。

- [ ] **Step 4: 检查最终差异和工作区边界**

Run:

```bash
git diff HEAD~2 --check
git status --short
```

Expected:

- `git diff --check` exit 0。
- 本任务文件已提交。
- 用户原有 `.superpowers/sdd/task-3-report.md` 修改仍保留且未进入本任务提交。
