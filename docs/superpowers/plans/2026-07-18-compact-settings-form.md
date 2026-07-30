# 紧凑设置表单 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 缩小设置浮层、为右侧表单提供一致内边距，并让所有设置文本框保持紧凑的固定尺寸。

**Architecture:** 应用层提供可测试的设置浮层布局构造函数。UI 层为 `TextBox` 增加可选固定尺寸，以保持 `FormRow` 既有的 IME 光标查询路径；`SettingsView` 使用该尺寸并收缩 `FormView` 的可用矩形。通用表单行、表单卡片与响应式规则保持不变。

**Tech Stack:** Rust、textora-app、textora-ui widgets、Cargo 测试。

## Global Constraints

- 产品名为 textora，Markdown 包名为 textora-markdown。
- `ui` 层不得依赖 `app` 状态；本次跨层不新增协议。
- 浮层首选尺寸为 680×480 逻辑像素，侧栏宽度为 152 逻辑像素。
- 右侧 `FormView` 四周内边距为 16 逻辑像素；四个文本框为 200×32 逻辑像素。
- 不改变通用表单行高、开关尺寸、输入校验、持久化、IME 定位或窄窗口响应式布局。
- 所有行为改动先写失败测试；避免 `.unwrap()`，并保持 `cargo fmt` 格式。

---

## 文件与职责

| 文件 | 职责 |
| --- | --- |
| `crates/app/src/settings_overlay.rs` | 定义可测试的紧凑设置浮层布局并在打开设置时使用它。 |
| `crates/ui/src/widgets/text_box.rs` | 支持文本框受逻辑固定尺寸约束，保持焦点与 IME 行为不变。 |
| `crates/ui/src/widgets/settings_view/widget.rs` | 缩小分类栏、为表单应用内边距，并配置四个设置文本框的固定尺寸。 |

### Task 1: 缩小设置浮层首选尺寸

**Files:**
- Modify: `crates/app/src/settings_overlay.rs:5-9, 38-54, 91-124`

**Interfaces:**
- Produces: `settings_overlay_layout() -> ui::OverlayLayout`。
- Consumes: `ui::OverlayLayout::resolve(Rect, dpi)` 验证逻辑尺寸。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn settings_overlay_uses_compact_preferred_dimensions() {
    assert_eq!(
        settings_overlay_layout().resolve(ui::Rect::new(0.0, 0.0, 1200.0, 800.0), 1.0),
        ui::Rect::new(260.0, 160.0, 680.0, 480.0),
    );
}
```

- [ ] **Step 2: 运行测试，确认失败**

运行：`cargo test -p textora-app settings_overlay_uses_compact_preferred_dimensions --lib`

预期：编译失败，因为 `settings_overlay_layout` 尚不存在。

- [ ] **Step 3: 写最小实现**

```rust
const SETTINGS_OVERLAY_PREFERRED_WIDTH_LOGICAL: f32 = 680.0;
const SETTINGS_OVERLAY_PREFERRED_HEIGHT_LOGICAL: f32 = 480.0;

fn settings_overlay_layout() -> ui::OverlayLayout {
    ui::OverlayLayout::Centered {
        preferred_size: (
            SETTINGS_OVERLAY_PREFERRED_WIDTH_LOGICAL,
            SETTINGS_OVERLAY_PREFERRED_HEIGHT_LOGICAL,
        ),
        min_margin: SETTINGS_OVERLAY_MIN_MARGIN_LOGICAL,
        max_width_ratio: SETTINGS_OVERLAY_MAX_WIDTH_RATIO,
        max_height_ratio: SETTINGS_OVERLAY_MAX_HEIGHT_RATIO,
    }
}
```

将 `open_settings_overlay` 的内联 `OverlayLayout::Centered` 替换为 `settings_overlay_layout()`。

- [ ] **Step 4: 运行测试，确认通过**

运行：`cargo test -p textora-app settings_overlay_uses_compact_preferred_dimensions --lib`

预期：1 个测试通过。

- [ ] **Step 5: 提交独立改动**

```bash
git add crates/app/src/settings_overlay.rs
git commit -m "fix(app): compact settings overlay"
```

### Task 2: 为 TextBox 添加可选固定尺寸

**Files:**
- Modify: `crates/ui/src/widgets/text_box.rs:31-88, 634-678, 940-980`

**Interfaces:**
- Produces: `TextBox::set_fixed_size_logical(width, height)`，在父级可用区域内左对齐、垂直居中。
- Consumes: 既有 `TextBox::ime_cursor_rect()`；设置页仍直接装箱 `TextBox`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn fixed_size_text_box_is_left_aligned_and_vertically_centered() {
    let theme = crate::theme::test_theme();
    let mut measure = NoopMeasure;
    let mut layout = LayoutCtx { ui_measure: None, measure: &mut measure, theme: &theme, dpi: 1.0 };
    let mut text_box = TextBox::with_id(WidgetId(90));
    text_box.set_fixed_size_logical(200.0, 32.0);

    text_box.set_rect(Rect::new(0.0, 0.0, 240.0, 56.0), &mut layout);

    assert_eq!(text_box.rect(), Rect::new(0.0, 12.0, 200.0, 32.0));
}
```

- [ ] **Step 2: 运行测试，确认失败**

运行：`cargo test -p textora-ui fixed_size_text_box_is_left_aligned_and_vertically_centered --lib`

预期：编译失败，因为 `set_fixed_size_logical` 尚不存在。

- [ ] **Step 3: 写最小实现**

向 `TextBox` 新增并初始化字段：

```rust
fixed_size_logical: Option<(f32, f32)>,

// TextBox::new
fixed_size_logical: None,
```

新增配置方法：

```rust
pub fn set_fixed_size_logical(&mut self, width: f32, height: f32) {
    self.fixed_size_logical = Some((width.max(0.0), height.max(0.0)));
}
```

在 `Widget for TextBox::set_rect` 中先约束布局矩形：

```rust
let layout_rect = self.fixed_size_logical.map_or(rect, |(width_logical, height_logical)| {
    let width = (width_logical * ctx.dpi).min(rect.w);
    let height = (height_logical * ctx.dpi).min(rect.h);
    Rect::new(rect.x, rect.y + (rect.h - height) * 0.5, width, height)
});
self.layout(layout_rect, ctx);
```

- [ ] **Step 4: 运行测试，确认通过**

运行：`cargo test -p textora-ui fixed_size_text_box_is_left_aligned_and_vertically_centered --lib`

预期：1 个测试通过。

- [ ] **Step 5: 提交独立改动**

```bash
git add crates/ui/src/widgets/text_box.rs
git commit -m "feat(ui): support fixed text box size"
```

### Task 3: 为设置表单添加内边距并配置紧凑文本框

**Files:**
- Modify: `crates/ui/src/widgets/settings_view/widget.rs:20-28, 239-311, 582-635, 833-1030`

**Interfaces:**
- Consumes: `TextBox::set_fixed_size_logical(200.0, 32.0)`。
- Produces: 680×480 设置页中的侧栏宽为 152，表单区域为 `Rect::new(192.0, 16.0, 472.0, 448.0)`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn settings_form_uses_compact_insets_and_sidebar_width() {
    let mut view = settings_fixture(SettingsCategory::Editor);
    let theme = crate::theme::test_theme();
    layout_settings_view(&mut view, &theme, Rect::new(0.0, 0.0, 680.0, 480.0));

    assert_eq!(view.sidebar_width, 152.0);
    assert_eq!(view.form_rect, Rect::new(192.0, 16.0, 472.0, 448.0));
}
```

- [ ] **Step 2: 运行测试，确认失败**

运行：`cargo test -p textora-ui settings_form_uses_compact_insets_and_sidebar_width --lib`

预期：断言失败，因为当前侧栏为 180 且表单没有内边距。

- [ ] **Step 3: 写最小实现**

```rust
const SETTINGS_SIDEBAR_WIDTH_LOGICAL: f32 = 152.0;
const SETTINGS_FORM_INSET_LOGICAL: f32 = 16.0;
const SETTINGS_TEXT_BOX_WIDTH_LOGICAL: f32 = 200.0;
const SETTINGS_CONTROL_HEIGHT_LOGICAL: f32 = 32.0;
```

在 `font_family_row`、`font_size_row`、`line_height_ratio_row` 与 `tab_width_row` 中，设置文字和 placeholder 后调用：

```rust
text_box.set_fixed_size_logical(
    SETTINGS_TEXT_BOX_WIDTH_LOGICAL,
    SETTINGS_CONTROL_HEIGHT_LOGICAL,
);
```

在 `SettingsView::set_rect` 中以 `form_inset = SETTINGS_FORM_INSET_LOGICAL * ctx.dpi` 计算：

```rust
self.form_rect = Rect::new(
    sidebar_width + SETTINGS_FORM_GAP_LOGICAL * ctx.dpi + form_inset,
    form_inset,
    (self.rect.w - sidebar_width - SETTINGS_FORM_GAP_LOGICAL * ctx.dpi - form_inset * 2.0)
        .max(0.0),
    (self.rect.h - banner_height - banner_gap - form_inset * 2.0).max(0.0),
);
```

`persistence_banner_rect` 继续复用 `form_rect` 的 X 与宽度，保持其现有的表单后置位置。

- [ ] **Step 4: 运行测试，确认通过**

运行：`cargo test -p textora-ui settings_form_uses_compact_insets_and_sidebar_width --lib`

预期：1 个测试通过。

- [ ] **Step 5: 执行相关回归测试**

运行：`cargo test -p textora-ui settings_view --lib`

预期：设置视图测试全部通过。

- [ ] **Step 6: 提交独立改动**

```bash
git add crates/ui/src/widgets/settings_view/widget.rs
git commit -m "fix(ui): compact settings form layout"
```

### Task 4: 格式化并执行完整验证

**Files:**
- Modify: 本任务不直接修改文件。

**Interfaces:**
- Consumes: Tasks 1–3 的设置浮层、文本框与设置页布局。
- Produces: 可复现的构建与项目验证结果。

- [ ] **Step 1: 运行格式检查**

运行：`cargo fmt --check`

预期：退出码为 0；若失败，运行 `cargo fmt` 后重新检查。

- [ ] **Step 2: 运行应用编译检查**

运行：`cargo check -p textora-app`

预期：退出码为 0。

- [ ] **Step 3: 运行项目完整验证**

运行：`./scripts/verify.sh`

预期：退出码为 0，全部检查通过。
