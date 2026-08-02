# Notora 设置设计

## 目标

参考 Textora 已验证的设置交互原则与分层方式，完成 Notora 设置从持久化模型、运行时应用、
模态输入、主题切换到失败恢复的闭环。Notora 拥有独立的配置文件、设置模型和设置界面，
不复用 Textora 的 `SettingsView`，也不读取或兼容 Textora 的 `.edit+` 配置。

## 范围

Notora 设置分为四组：

- `Appearance`：主题模式；
- `Editor`：字体、字号、行高、自动换行、行号和 Tab 宽度；
- `Interface`：状态栏与运行时 Tab 上限；
- `Workspace`：自动保存延迟与 catalog 备份保留数。

Notora 在 `notora-app` 中组合自己的四分类设置界面，不引入反射式设置 schema。
两个产品只共享 `FormView`、`FormSection`、`FormRow`、`Button`、`Switch`、`TextBox`
等基础控件以及主题语义，不共享设置字段、分类或业务 action，也不让 `ui` 理解 Notora 的
workspace、catalog 或 runtime 类型。

## 分层与数据流

```text
settings.toml
  -> notora_app::ProductSettings
  -> 显式映射到 ui::Settings / Notora runtime policy
  -> SettingsOverlayInput（纯展示输入）
  -> notora_app::NotoraSettingsView

控件动作
  -> ProductSettingsUpdate
  -> NotoraAction / NotoraEffect
  -> 即时更新运行时
  -> PersistenceWorker 串行原子保存
  -> SettingsPersistenceCompleted
  -> Saved 或 SaveFailed（可重试）
```

`ui` 只提供产品无关的基础控件与主题令牌。Notora 的设置输入、持久化展示状态、产品设置
DTO、保存路径、后台 worker 和应用状态全部留在 `notora-app`；Textora 的设置页面及输入
协议保持不变。

## 持久化语义

- 合法修改先应用到当前运行时，再异步保存，不因 I/O 失败回滚用户刚刚选择的值。
- 后台保存必须同时报告成功和失败，保证一次失败后的重试成功能够清除错误提示。
- 设置保存结果与 session 保存失败使用不同的类型化事件，禁止再把两者折叠成无法识别来源的
  `PersistenceFailed`。
- 设置页收到失败状态后显示共享的“当前修改尚未保存”提示与重试入口；重试保存当前完整
  `ProductSettings` 快照。
- 保存继续使用同目录临时文件、`sync_all` 和原子替换；退出时按现有 worker shutdown 顺序
  刷新队列。

## 输入与焦点

设置弹层打开时是严格模态：pointer、滚轮、键盘和 IME 事件即使没有产生产品动作，也必须
被产品层消费，不能继续传给 editor runtime。文本输入中的 `TextEdited` 只更新控件内部值，
合法提交才产生类型化设置更新；无效提交保持在控件中，不修改运行时或持久化快照。

`Escape` 沿用应用级 overlay 关闭协议，关闭后焦点恢复由现有 reducer 负责。

## 主题

- `Light`、`Dark` 和 `System` 统一通过 `ui::Theme::resolve_builtin` 解析；
- `System` 使用窗口当前外观，并响应 `WindowEvent::ThemeChanged`；
- Notora 外壳、设置页、对话框和模态遮罩只选择 `ApplicationTheme` / `SettingsTheme`
  语义角色，不直接组合底层 palette 或硬编码黑色遮罩；
- 设置变化后同时更新 editor runtime 的 settings 与 theme，并请求重绘。

## 验证

- 设置 DTO：默认值、版本、未知字段、非法运行参数、原子 round-trip；
- 持久化：后台成功/失败结果、失败展示、重试成功清除提示、关闭前刷新最新快照；
- 输入：设置弹层内未产生 action 的字符、IME 和 pointer 事件仍阻断 editor；
- 主题：强制深浅色、跟随系统、系统外观变化和共享语义令牌；
- 定向执行 Notora、UI 与 appkit-shell 测试；最终运行 `cargo fmt` 和
  `./scripts/verify.sh`。
