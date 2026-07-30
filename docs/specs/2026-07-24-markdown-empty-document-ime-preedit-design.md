# Markdown 空文档 IME Preedit 回归设计

## 问题

新建空 Markdown WYSIWYG 文档的 generation 为 0，应用不会发送
`UpdateSource`。IME 组合输入依次发送 `SetCursorByte(0)` 与
`SetPreedit`，但旧的 `PreviewEngine.edit_source` 为 `None`，
`empty_source_line_preedit_render_data()` 因而无法生成组合文字绘制数据。

空文档光标修复已在 `MarkdownEditorView::new()` 中建立
`edit_source = Some("")` 的初始化不变量，理论上同时覆盖此问题，但当前测试只覆盖
“已有内容后的末尾空行”，没有覆盖新建空文档首次组合输入。

## 方案

添加编辑器级回归测试，严格复现真实消息顺序：

1. 创建内容为空、generation 为 0 的文档与 `MarkdownEditorView`。
2. 不调用 `set_source`，依次发送 `SetCursorByte(0)` 和非空 `SetPreedit`。
3. 渲染一次，断言 DrawList 中存在完整的 preedit 文本。
4. 断言查询到的组合光标位于 preedit 文本末端，验证文字和光标使用同一排版结果。

若测试已通过，不增加新的生产分支；这表示空光标修复已经消除了共同根因，只补齐回归覆盖。
若测试失败，则仅在 Markdown 编辑器空行 preedit 数据生成链路中修复已定位的失败点，
不修改应用层 IME 协议，也不为 generation 0 强制发送 `UpdateSource`。

## 验证

- 定向运行新回归测试。
- 运行 `cargo test -p textora-markdown --lib`。
- 运行 `cargo check -p textora-markdown`。

