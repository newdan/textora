# 通用应用主题设计

## 目标

把 Textora 已形成的应用视觉语言沉淀到 `ui` 层，使 Textora、Notora 和后续产品通过同一套语义令牌绘制应用外壳。产品层只选择“这里是什么视觉角色”，不再自行组合底层色板或硬编码遮罩颜色。

## 现状与问题

`ui::Theme` 已经统一持有 `ColorPalette`、编辑器、Markdown、小说和思维导图主题，但应用外壳仍直接读取 `ColorPalette`：

- Textora 的通用 shell 自行从 `shadow` 推导模态遮罩；
- Notora 自行选择三栏背景、弹层表面、主次文本和固定黑色遮罩；
- `SettingsTheme` 直接从色板派生，与应用外壳没有共同的语义入口；
- Notora 的 `System` 模式固定解析为深色，没有复用 Textora 的主题解析规则。

这些做法虽然引用同一份颜色值，但没有共享同一份设计决策。修改 Textora 的视觉层级时，Notora 不会自动获得一致结果。

## 设计

在 `ui::theme` 中新增纯数据 `ApplicationTheme`。它由 `Theme` 派生，表达以下通用角色：

- 窗口、导航、内容、编辑区和浮层表面；
- 普通悬停、普通选中和导航选中状态；
- 主文本、次文本、反色文本和导航选中文本；
- 弱分隔线、强边框、控件表面与控件边框；
- 强调、危险和警告反馈；
- 统一模态遮罩。

`ApplicationTheme` 不保存产品状态，不依赖 `DocumentView`、Workspace 或 app crate。它只把 `ColorPalette` 与 `EditorTheme` 映射为稳定语义，并在 gamma correction 完成后构造。

主题文件继续配置现有底层字段。应用令牌是派生视图，不新增 TOML 必填项，因此旧主题文件无需迁移。组件尺寸、圆角和留白仍由组件自己的具名逻辑尺寸常量负责；本次不把布局参数混入颜色主题。

`SettingsTheme` 改为从 `ApplicationTheme` 派生，以保证设置界面与应用外壳共享表面、边框、文本和强调色的映射规则。

## 产品接入

### Textora

- 通用 shell 使用 `ApplicationTheme::modal_scrim` 和 `divider`；
- 默认活动主题对直接使用注册表内置 ID，不再设置不存在的 `spec-light/spec-dark` 后依赖 fallback。

### Notora

- 三栏外壳、工具区、菜单、对话框和提示层使用 `ApplicationTheme` 语义令牌；
- 所有模态层使用与 Textora 相同的遮罩令牌；
- 主题模式通过共享解析规则处理，`System` 使用窗口当前外观，并响应系统主题变化。

## 验证

- `ui` 单元测试锁定底层色板到应用令牌、应用令牌到设置令牌的映射；
- Notora 单元测试覆盖浅色、深色和跟随系统的解析；
- Textora shell 测试与 Notora render 测试断言遮罩及外壳表面来自通用令牌；
- 分阶段执行相关 crate 编译，最终运行 `./scripts/verify.sh`。
