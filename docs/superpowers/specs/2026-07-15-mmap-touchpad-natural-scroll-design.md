# mmap 触摸板自然滚动方向修复

> 日期：2026-07-15
> 状态：已确认，待实现

## 1. 问题与根因

mmap 画布通过 `winit::event::MouseScrollDelta::PixelDelta` 接收触摸板滚动。winit 的约定是正值表示被滚动内容向右、向下移动。

画布视口的 `scroll.x/y` 是内容平移量：增大时，`content_to_screen` 会让内容向左、向上移动。因此自然滚动输入必须转换为 `PanBy(-delta.x, -delta.y)`。

当前实现将 `PixelDelta` 转换为 `(delta.x, -delta.y)`，导致横向触摸板手势与 macOS 自然滚动直觉相反；纵向符号已经正确。

## 2. 设计

- 保持 `CanvasViewportSession`、滚动条拖拽、轨道翻页和缩放的现有坐标约定不变。
- 仅修正 `canvas_pan_delta` 对 `PixelDelta.x` 的符号，使两个轴统一为 `(-x, -y)`。
- `LineDelta` 的现有转换保持不变，避免改变鼠标滚轮的既有行为。
- Shift+触摸板横向转换继续复用修正后的画布平移增量。

## 3. 测试与验收

在 `crates/app/src/app_scroll.rs` 增加或调整集成回归测试：

- 从横纵均非零的画布位置接收正向横向、负向纵向 `PixelDelta`。
- 断言横向视口位置减少、纵向视口位置增加，分别对应自然触摸板手势在两个轴上的内容移动。
- 保留 `AppEffect::REDRAW` 断言，并运行 app 相关测试、格式检查和项目完整验证。

## 4. 范围外

不修改 `ui::scrollbar` 的滑块几何或点击/拖拽方向，不修改普通编辑器、侧栏、目录和 WYSIWYG 的滚动转换，也不改变画布内部 `scroll` 的符号语义。
