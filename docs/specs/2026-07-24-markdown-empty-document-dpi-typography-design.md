# Markdown 空文档 DPI 排版指标修复设计

## 问题

Markdown WYSIWYG 正常文本行的字号来自 `MarkdownStyle`，已经按窗口 DPI
转换为物理像素。完全空文档没有任何 `FlatLine`，因此
`empty_source_line_metrics()` 会回退到 `PreviewEngine.base_font_size` 和
`base_line_height`。

这两个字段保存的是应用通过 `SetRenderSettings` 传入的逻辑像素值。当前回退路径
直接将逻辑字号作为光标高度和 IME preedit 字号，在高 DPI 屏幕上会偏小。例如
逻辑字号 15px、DPI 2x 时，正文使用 30px，而初始空文档光标和 preedit 仍使用
15px。

## 方案

在 `PreviewEngine` 中增加“最近一次渲染使用的物理正文排版指标”：

- `rendered_body_font_size`
- `rendered_line_height`

`PreviewEngine::render()` 每次收到 `MarkdownStyle` 时，立即用
`style.body_font_size` 和 `style.line_height` 更新这两个字段。它们只表达本帧实际
渲染空间中的物理尺寸，不替代保存逻辑设置的 `base_font_size` 和
`base_line_height`。

完全空文档找不到相邻排版行时，`empty_source_line_metrics()` 使用上述物理指标。
已有相邻文本行时仍沿用 `FlatLine.font_size` 和 `FlatLine.rect.h`，不改变标题、
列表、尾随空行等现有行为。

这一修改同时覆盖：

- 初始空文档光标高度。
- 初始空文档 IME preedit 字号。
- `CursorScreenPos` 查询返回的光标矩形。

## 数据流

```text
Settings 逻辑字号
  → UiMetrics 按 DPI 转换
  → MarkdownEditorView 构造 MarkdownStyle
  → PreviewEngine::render 记录物理正文指标
  → 空文档排版指标回退
  → 光标、preedit、CursorScreenPos
```

## 测试

添加一个 2x DPI 回归测试，创建未调用 `set_source` 的新空编辑器：

1. 设置光标为 byte 0，并发送非空 IME preedit。
2. 以 DPI 2.0 渲染。
3. 断言光标高度为逻辑字号的两倍。
4. 断言 preedit `TextLayout.font_size` 与光标高度一致。
5. 断言 `CursorScreenPos(0)` 返回相同的物理高度。

随后运行：

- 新增定向测试。
- `cargo test -p textora-markdown --lib`。
- `cargo fmt --all --check`。
- `cargo check -p textora-markdown`。

