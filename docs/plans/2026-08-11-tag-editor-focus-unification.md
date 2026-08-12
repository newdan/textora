# 标签编辑器焦点统一计划

## 问题

`TagEditorWidget` 自行维护文本、编辑状态、IME 预编辑和光标位置；
`EditorPaneChrome` 又维护 `tag_editor_active` 并手工生成焦点请求。这让命中测试承担了
焦点语义，最终把标签区域内的 `MouseMove` 错误解释成输入框激活。

## 审计结论

- 标题、搜索框和设置表单均使用 `TextBox` 管理文本输入状态。
- 按钮、列表、开关和分隔条的焦点状态用于键盘可达性，不属于重复文本输入实现。
- 标签编辑器是当前唯一同时手写文本编辑、IME、光标和焦点状态的 UI 组件。

## 目标状态

1. `TagEditorWidget` 内嵌无边框 `TextBox`，标签 chips 与候选项仍由组合组件管理。
2. 鼠标、键盘、IME、选择、光标和焦点请求统一委托给 `TextBox`。
3. `TagEditorWidget` 通过 `Widget::set_keyboard_focus` 接收焦点，不再暴露 `editing` 状态。
4. `EditorPaneChrome` 只负责坐标转换与事件路由，不再维护 `tag_editor_active` 或手工构造焦点请求。
5. `NotoraShell` 保留 `FocusTarget` 作为应用级唯一焦点仲裁状态。

## 验证

- hover 标签区域不请求焦点，左键点击才请求焦点。
- 标签输入、退格移除、回车提交、Escape 取消和候选选择行为保持不变。
- 只有获得焦点的标签 `TextBox` 消费 IME，并提供候选窗位置。
- 文档刷新保留当前草稿，切换文档清理瞬态焦点状态。
- 运行相关单元测试、`cargo check` 和 `./scripts/verify.sh`。
