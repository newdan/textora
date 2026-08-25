# Textora 控件焦点与输入事件路由修复计划

日期：2026-08-24

## 问题

加密笔记创建对话框在“确认密码”获得视觉焦点后，仍会把键盘和 IME 输入写入“密码”。原因不是字段状态绑定错误，而是复合控件按固定顺序向两个 `TextBox` 广播输入事件，同时 `TextBox` 没有拒绝未聚焦的键盘与 IME 事件。

## 不变量

1. 一个复合控件只持有一个键盘焦点目标。
2. `KeyDown`、`ImePreedit`、`ImeCommit`、`ImeEnable` 和 `ImeDisable` 只派发给该目标。
3. 指针按命中结果派发；拖拽中的指针事件只派发给捕获者。
4. `TextBox` 未聚焦时不得因键盘或 IME 事件修改正文或产生编辑 action。
5. 视觉焦点、IME 光标目标、实际内容修改目标和 action 的 `WidgetId` 必须一致。

## 子任务一：通用 TextBox 边界

- 在 `Widget::on_event` 契约中记录键盘与 IME 的单目标路由要求。
- 先增加未聚焦 `TextBox` 拒绝字符键、提交键和 IME 输入的回归测试。
- 在 `TextBox::on_event` 入口拒绝未聚焦键盘与 IME 输入。
- 审计直接调用 `TextBox::on_event` 的复合控件，确保调用前已设置焦点。

## 通用设施设计

通用能力位于 `ui::core::child_event_router`，不依赖 app 状态或具体控件类型：

- `ChildEventRouter<T>` 维护唯一焦点目标、指针捕获目标和 Hover 目标；`T` 是容器内部的稳定目标标识。
- `ChildEventRoute<T>` 描述原始事件的唯一接收者、Hover 切换时的旧目标以及取消事件的广播要求。
- `FocusDirection` 与 `next_focus_target` 统一处理 Tab / Shift+Tab 的前后遍历和首尾闭环。
- 容器仍负责布局、命中测试、坐标转换、当前可见/可用候选列表和业务 action 映射。
- 叶控件仍必须拒绝不符合自身焦点状态的键盘与 IME，形成“父容器单目标路由 + 叶控件不变量防御”的双层契约。

路由器不能持有 `Widget` 引用，也不能读取 `DocumentView`、应用状态或业务输入；因此它可以同时用于文本框、数值输入、按钮、表单和 Modal，而不破坏 UI 分层。

## 子任务二：加密笔记对话框

- 使用单一枚举表示密码、确认密码、提交和取消四种子控件目标，并交给通用路由器维护。
- 所有焦点切换统一经过一个入口，并同步到每个子控件。
- 指针事件只发给命中的子控件，键盘与 IME 事件只发给当前焦点目标。
- 密码输入按 Enter 后转到确认密码；确认密码按 Enter 时仅在表单有效时提交。
- Tab 和 Shift+Tab 在当前模式可见、可用的控件间循环；解锁模式跳过确认密码。
- 对话框关闭、重新打开和提交失败后的焦点恢复复用同一入口。

## 子任务三：通用容器迁移

- `InlineGroup` 使用通用路由器派发键盘、IME、指针和 Hover 生命周期事件。
- `FormRow`、`FormSection`、`FormView` 逐层使用通用路由器；数值输入等设置控件通过表单层自然获得同一协议。
- `SettingsView` 只保留分类、表单和持久化提示区的目标映射，不再维护独立的指针与 Hover 状态。
- `ModalFrame` 使用通用路由器形成 Modal 内部的唯一输入目标，并保留 Escape 关闭这一专用策略。
- 子控件自行建立的拖拽捕获可通过 `set_pointer_capture_target` 同步到父容器；该捕获只影响指针事件，不得改变键盘或 IME 目标。

## 验证

```bash
cargo test -p textora-ui text_box
cargo test -p textora-ui child_event_router
cargo test -p textora-ui widgets::form
cargo test -p textora-ui modal_frame
cargo test -p textora-ui encrypted_note_dialog
cargo test -p notora-app
cargo fmt --check
./scripts/verify.sh
```

测试和日志不得输出密码或确认密码明文。
