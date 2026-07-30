# mmap 触摸板自然滚动修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修正 mmap 画布触摸板横向自然滚动的符号，并用横纵双轴回归测试锁定正确行为。

**Architecture:** 保持 `CanvasViewportSession` 的 `scroll` 坐标约定不变，只在 app 输入边界修正 `PixelDelta` 到 `PanBy` 的转换。`LineDelta`、画布滚动条和其他滚动区域不变。

**Tech Stack:** Rust、Cargo workspace、winit `MouseScrollDelta`、`edit-plus-app` 单元测试。

## Global Constraints

- 全程使用中文沟通；产品名为 `textora`，Markdown crate 包名为 `textora-markdown`。
- 保持 `ui → app → markdown` 依赖方向，不让 `ui` 访问 app 状态。
- 先写回归测试并确认失败，再写最小生产修复；禁止无关重构。
- 遵守 Rust 命名、错误处理和 `cargo fmt` 规范；不新增无意义魔法值。
- 修改后至少运行相关 app 测试、`cargo fmt --all -- --check` 和 `./scripts/verify.sh`。

---

### Task 1: 修正 mmap 触摸板双轴自然滚动

**Files:**
- Modify: `crates/app/src/app_scroll.rs:14-25`，统一 `PixelDelta` 的横纵转换符号。
- Test: `crates/app/src/app_scroll.rs`，将既有双轴测试收敛为命名明确的自然触摸板回归测试。

**Interfaces:**
- Consumes: `canvas_pan_delta(MouseScrollDelta, bool) -> ui::canvas::CanvasPoint`、现有 `app_with_prepared_canvas_viewport()` 测试夹具。
- Produces: `PixelDelta(x, y)` 转换为 `CanvasPoint::new(-x, -y)`；`LineDelta` 和 Shift 路径保持现状。

- [x] **Step 1: 写失败的双轴回归测试**

将既有 `canvas_pixel_scroll_pans_both_axes` 测试替换为命名明确的：

```rust
#[test]
fn canvas_pixel_scroll_follows_natural_touchpad_on_both_axes() {
    let mut app = app_with_prepared_canvas_viewport();
    app.apply_canvas_viewport_action(CanvasViewportAction::PanBy(
        ui::canvas::CanvasPoint::new(200.0, 200.0),
    ));
    let before = app
        .workspace
        .active_entry()
        .expect("test canvas tab must be active")
        .canvas_viewport
        .snapshot()
        .expect("prepared canvas viewport must retain a snapshot");

    assert_eq!(
        app.handle_scroll(MouseScrollDelta::PixelDelta(PhysicalPosition::new(36.0, -72.0))),
        AppEffect::REDRAW
    );

    let after = app
        .workspace
        .active_entry()
        .expect("test canvas tab must remain active")
        .canvas_viewport
        .snapshot()
        .expect("canvas viewport snapshot must remain available");
    assert!((after.scroll.x - (before.scroll.x - 36.0)).abs() < 0.001);
    assert!((after.scroll.y - (before.scroll.y + 72.0)).abs() < 0.001);
}
```

- [x] **Step 2: 运行测试确认它因横向符号错误而失败**

运行：

```bash
cargo test -p textora-app --lib canvas_pixel_scroll_follows_natural_touchpad_on_both_axes
```

预期：测试失败，纵向断言通过但横向断言失败；现有实现把 `PixelDelta.x = 36.0` 转成 `scroll.x + 36.0`，测试要求自然滚动语义下的 `scroll.x - 36.0`。

- [x] **Step 3: 写最小生产修复**

在 `canvas_pan_delta` 中仅将 `PixelDelta` 分支改为：

```rust
MouseScrollDelta::PixelDelta(position) => {
    CanvasPoint::new(-(position.x as f32), -(position.y as f32))
}
```

不改 `LineDelta` 分支、不改 `CanvasViewportSession`、不改 `ui::scrollbar`。

- [x] **Step 4: 运行新增测试确认通过**

运行同一命令：

```bash
cargo test -p textora-app --lib canvas_pixel_scroll_follows_natural_touchpad_on_both_axes
```

预期：PASS。

- [x] **Step 5: 运行相关滚动画面测试，确认纵向和 Shift 路径没有回归**

运行：

```bash
cargo test -p textora-app --lib canvas_pixel_scroll
cargo test -p textora-app --lib canvas_shift_scroll_converts_vertical_line_delta_to_horizontal_pan
cargo test -p textora-app --lib canvas_command_scroll_zooms_at_mouse_anchor_without_panning
```

预期：全部 PASS；触摸板双轴测试验证 `scroll.x` 减少、`scroll.y` 增加，LineDelta 与缩放行为保持原有结果。

- [x] **Step 6: 格式化并执行项目验证**

运行：

```bash
cargo fmt --all
cargo fmt --all -- --check
./scripts/verify.sh
```

预期：格式检查通过，项目验证脚本退出码为 0。

- [x] **Step 7: 提交实现**

运行：

```bash
git add crates/app/src/app_scroll.rs
git commit -m "fix(mmap): align touchpad natural scrolling"
```

预期：只提交 `app_scroll.rs` 的生产代码与回归测试改动。
