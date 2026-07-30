# Textora 设置界面与基础 UI 控件扩充设计

## 1. 目标

为 Textora 建立一组可复用的基础 UI 控件和通用容器，并用它们实现窗口内的单例设置浮层。设置浮层覆盖在编辑器之上；背景编辑器继续绘制，以便用户观察主题、字体和排版设置的即时效果，但在浮层关闭前不接收任何输入。

第一期接入真实设置并持久化，覆盖以下分类：

- 外观：主题模式、字体、字号、行高比例。
- 编辑器：自动换行、行号、Tab 宽度。
- 界面：侧边栏/标签页模式、状态栏显示。
- 同步：本机 Syncthing 控制连接配置和状态。

## 2. 非目标

- 不创建操作系统级独立设置窗口。
- 不建立任意层叠、动画驱动的完整 Overlay 框架。
- 不引入反射式设置 Schema、万能 `SettingValue` 或自动表单生成器。
- 不让 `ui` crate 访问 `DocumentView`、app 状态、Syncthing REST DTO、Keychain 或工作线程。
- 不在本功能中实现 Syncthing 协议、管理 Syncthing 进程或修改远端 Syncthing。
- 不为 Syncthing 创建业务专用的基础控件或布局容器。

## 3. 已确认的产品决策

- 设置界面是同一应用窗口内的居中模态浮层，不是编辑区页面或独立窗口。
- 设置浮层打开时，背景编辑器完全禁止交互。
- 同一时间首期只允许一个交互式 Overlay。
- 普通设置在合法值提交后即时生效并持久化，不提供全局“应用/取消”按钮。
- API Key 在设置页完成首次配置和后续更新，但从不回显已保存明文。
- Switch 用于即时启用/禁用功能；Checkbox 用于表单选择、附加选项或多选项。
- Label、Button、TextBox 等基础控件通过组合表达业务语义，不按业务名称派生控件类型。
- 设置界面整体参考当前 macOS System Settings 的视觉语言，但不依赖 AppKit，也不做像素级复制。

## 4. 总体架构

```text
UiShell
├── BaseContent（现有 Dock / 编辑器）
└── ActiveOverlay（可选）
    └── ModalFrame
        └── SettingsView
            ├── CategoryNavigation
            └── FormView
                └── FormSection[]
                    └── FormRow[]
                        ├── Label
                        └── 任意基础控件或 InlineGroup
```

现有 `UiShell::overlays`、`push_overlay`、`pop_overlay` 和优先事件分发是实现基础。本功能不平行创建第二套宿主，而是把现有机制扩充为明确的 Overlay 条目、布局策略、输入策略和关闭策略。

`SettingsView` 是业务视图，只负责组装通用容器和基础控件，并把通用控件动作映射为设置动作。Overlay 的遮罩、尺寸、事件拦截和焦点恢复不属于 `SettingsView`。

## 5. macOS 风格视觉原则

设置界面参考当前 macOS System Settings 的信息层级和控件气质，重点复用其设计原则，而不是复制原生窗口结构：

- 左侧使用固定宽度分类侧栏，分类项左对齐，选中项使用圆角背景块。
- 右侧使用清晰的页面标题、充足留白和分组设置卡片。
- 同一分组中的设置行共享圆角 Surface，行之间使用克制的细分隔线。
- Label 使用系统 UI 字体族，并通过字号、字重和间距建立层级；避免依赖大量颜色区分内容。
- 状态主要通过图标和明确文案表达，不把成功、警告或错误固化成 Label 颜色类型。
- 输入框、按钮、Switch 和 Checkbox 采用接近 macOS 的紧凑比例、圆角和焦点环。
- Button 允许主题提供普通、强调和警示背景；强调来自样式，不来自业务控件类型。
- hover、pressed、selected 和 focus 状态保持轻量，不使用夸张阴影或高饱和装饰。

所有视觉值由 Theme 中的语义 Token 提供，包括：

```text
modal_surface
sidebar_surface
section_surface
section_border
separator
control_surface
control_border
focus_ring
accent
text_primary
text_secondary
```

尺寸、圆角、留白和分隔线宽度使用具名逻辑尺寸常量。亮色与暗色主题分别定义 Token，统一经过现有 DPI 和颜色空间处理。

该风格在所有平台保持一致：macOS 使用平台系统 UI 字体，Windows 和 Linux 使用项目已有的平台 UI 字体回退。实现继续使用 Textora 的 wgpu UI，不引入平台专属设置控件。

## 6. 通用 Overlay 模型

```rust
struct OverlayEntry {
    widget: Box<dyn Widget>,
    layout: OverlayLayout,
    input_policy: OverlayInputPolicy,
    dismiss_policy: DismissPolicy,
    restore_focus: KeyboardFocusTarget,
}
```

第一期支持两种输入策略：

```rust
enum OverlayInputPolicy {
    Modal,
    PassThrough,
}
```

- `Modal` 拦截鼠标、滚轮、键盘和 IME。即使子控件没有产生 Action，事件也不得继续下传 Dock 或编辑器。
- `PassThrough` 留给 Tooltip 等非交互覆盖物；交互式设置浮层不使用该策略。

关闭策略：

```rust
enum DismissPolicy {
    ExplicitOnly,
    EscapeOrExplicit,
    EscapeBackdropOrExplicit,
}
```

设置页使用 `EscapeOrExplicit`：关闭按钮或 Escape 关闭，点击遮罩不关闭。关闭时恢复打开前的键盘焦点和 IME 状态。

设置页使用通用居中布局：

```rust
OverlayLayout::Centered {
    preferred_size,
    min_margin,
    max_width_ratio,
    max_height_ratio,
}
```

所有布局值使用具名逻辑尺寸常量并统一按 DPI 缩放。小窗口中 ModalFrame 限制在可用区域内，内容由内部 FormView 滚动。

## 7. 基础控件

### 7.1 Label

Label 支持：

- 文本。
- 可选前置图标和后置图标。
- 字体、字号、前景色、对齐和换行等排版样式。

Label 不定义普通、成功、警告或错误等业务枚举。业务状态由调用方选择图标和文案。可点击的复制图标不是 Label 的内建行为，而是独立 Icon Button，通过 InlineGroup 与 Label 组合。

### 7.2 Button

Button 支持：

- 可选文字。
- 可选图标。
- `ButtonStyle`。
- enabled、hovered、pressed 和 selected 状态。

`ButtonStyle` 定义不同交互状态下的前景、背景、边框、圆角和内边距。强调按钮、警示按钮、中性按钮和图标按钮只是主题提供的样式预设，不形成 `ButtonVariant` 业务枚举。

点击在鼠标于按钮内部按下、随后于内部释放时产生；MouseDown 不立即触发业务动作。分类导航项复用带 selected 状态的 Button。

### 7.3 TextBox

现有 TextBox 的单行编辑、选区、IME、剪贴板和光标能力继续复用，并扩充为可被通用容器承载的 Widget。

新增回显模式：

```rust
enum EchoMode {
    Plain,
    Masked,
}
```

- Plain 输入可以产生编辑和提交动作。
- Masked 输入不逐键向业务层发送明文，只在用户显式提交时上送敏感值。
- API Key 保存成功后立即清空 TextBox；app 回注给 UI 的只有“是否已配置”。

TextBox 的文本变化、提交和焦点请求改走统一 Action，不通过业务回调传播。剪贴板访问继续通过由 app 注入、在 `ui` 定义的抽象接口完成，不进入设置业务动作。

### 7.4 Switch 与 Checkbox

Switch 和 Checkbox 是两个独立视觉控件，但复用相同的布尔切换、焦点和键盘行为。

- Switch 表达立即生效的开关设置。
- Checkbox 表达选择、附加选项或多选项。
- 两者都输出带控件身份和新值的 Toggle Action。

## 8. 控件身份、Action 与焦点

所有可交互基础控件持有稳定 `WidgetId`。通用控件动作包含来源身份：

```rust
enum ControlAction {
    Activated { id: WidgetId },
    Toggled { id: WidgetId, checked: bool },
    TextEdited { id: WidgetId, value: TextPayload },
    TextCommitted { id: WidgetId, value: TextPayload },
    FocusRequested { id: WidgetId },
}
```

`SettingsView` 把 `WidgetId` 映射为 `SetWordWrap`、`SetFontSize`、`TestSyncthingConnection` 等业务动作。基础控件与 Form 容器不知道设置字段名称。

普通文本使用 String；敏感文本使用 `SensitiveText`。`SensitiveText` 包装 `zeroize::Zeroizing<String>`，手工实现脱敏 Debug，禁止在日志和派生调试输出中打印内容。敏感 Action 的所有副本在销毁时清理内存。

焦点由 SettingsView 统一管理：

- 点击可交互控件时请求焦点。
- Tab 和 Shift+Tab 按当前可见、启用控件的顺序移动。
- 键盘和 IME 只发送给当前焦点控件。
- 切换分类时，焦点移动到新分类的第一个可交互控件；若不存在，则落在 SettingsView 容器。

## 9. 通用 Form 容器

### 9.1 FormView

- 负责纵向内容布局、裁剪和滚动。
- 分类切换后滚动位置恢复到顶部。
- 不解释设置值或控件 Action。

### 9.2 FormSection

- 包含标题 Label、可选说明 Label 和 FormRow 集合。
- 支持通用 Surface 样式，包括背景、边框、圆角和内部行分隔线。
- 只承担分组视觉与布局，不解释设置值或业务状态。

### 9.3 FormRow

- 左侧承载名称 Label 和可选说明 Label。
- 右侧承载任意 Widget 或 InlineGroup。
- 宽度充足时左右排列；低于具名响应式阈值时上下排列。

### 9.4 InlineGroup

- 横向排列多个基础控件。
- 支持具名间距、交叉轴对齐和剩余空间分配。
- 用于 Label + 复制 Button、Masked TextBox + 保存 Button、状态 Label + 操作 Button 等组合。

这些容器是结构性布局组件，不是设置 Schema。SettingsView 显式拥有和更新具体控件，避免万能值枚举、反射和隐式业务绑定。

## 10. 设置页内容

ModalFrame 包含顶部 Header 和主体：

```text
Header
├── Label：设置
└── Icon Button：关闭

Body
├── CategoryNavigation
└── FormView
```

CategoryNavigation 包含外观、编辑器、界面和同步四个纵向 Button。右侧 FormView 只显示当前分类。

分类侧栏、页面标题、FormSection Surface 和设置行间距遵循第 5 节的 macOS 风格 Token。设置浮层仍是 Textora 窗口内的 ModalFrame，不模拟独立的 macOS 系统设置窗口标题栏。

选择型设置使用带 selected 状态的 Button 组合：

```text
主题模式：  [跟随系统] [浅色] [深色]
视图模式：  [侧边栏]   [标签页]
```

Switch 用于自动换行、行号、状态栏和其他即时布尔设置。字体、字号、行高比例和 Tab 宽度使用 TextBox，在提交点完成校验。

## 11. Syncthing 设置组合

同步分类不定义 `SyncthingSettingsSection` 或 Syncthing 专用控件，使用通用 FormSection、FormRow、InlineGroup 和基础控件组装：

- loopback 地址：Label + TextBox。
- API Key：配置状态 Label + Masked TextBox + 保存/更新 Button。
- 连接测试：Button + 结果 Label。
- 本机 Device ID：Label + 复制 Icon Button。
- Syncthing 版本：Label。
- 打开本机 Web UI：带图标 Button。
- 断开 Textora 控制连接：强调背景 Button。

API Key 已配置时，Masked TextBox 保持为空，placeholder 提示“输入新 Key 可更新”。空提交不覆盖已有 Key。

断开操作不建立第二层 Overlay。首次点击后，当前 FormRow 原地显示确认 Label、确认 Button 和取消 Button。确认断开执行以下动作：

- 停止 Textora 的 Syncthing REST 控制请求和事件订阅。
- 删除安全存储中的 API Key，使下次启动不自动重连。
- 保留 loopback 地址。
- 不停止 Syncthing 进程。
- 不修改 Syncthing 的设备、资料库或文件。
- 不删除 Textora 已保存的资料库映射；映射进入未连接状态。

首版仅接受明文 HTTP loopback 地址，包括 `127.0.0.1`、`localhost` 和 IPv6 loopback。拒绝局域网、公网主机和非 HTTP 地址，避免 API Key 离开本机。

## 12. 单向数据流

```text
app 状态
→ SettingsViewInput（纯数据）
→ SettingsView 更新基础控件
→ ControlAction
→ SettingsViewAction
→ app 修改状态或执行 AppEffect
→ 新 SettingsViewInput
```

`SettingsViewInput` 可以包含具体设置值和 Syncthing 连接视图状态，但不能包含 app 状态结构体、REST DTO、Keychain 句柄或工作线程对象。

提交规则：

- Switch、Checkbox、主题模式和视图模式点击后立即提交。
- 字体、字号、行高比例、Tab 宽度和 loopback 地址在 Enter 或失焦时校验并提交。
- 输入过程中的空字符串、不完整小数等中间状态只保留在 TextBox 内部。
- API Key 仅通过保存/更新按钮显式提交。
- 校验失败时保留输入和焦点，并通过带图标 Label 显示原因。

## 13. 持久化与异步状态

UI 不直接读写 `settings.toml`。合法设置先应用到运行时，再由 app 产生持久化 Effect。

- 保存成功不持续显示提示。
- 保存失败不回滚已经生效的设置。
- 失败时在设置页顶部显示“当前修改尚未保存”Label 和重试 Button。

Syncthing 连接状态使用互斥 enum，而不是多个 bool：

```rust
enum SyncthingConnectionViewState {
    Disconnected,
    Testing,
    Connected,
    UnsupportedVersion,
    AuthenticationFailed,
    Unavailable,
}
```

这些状态只决定通用 Label 的图标/文案和相关控件是否启用。app 从同步领域状态构造纯 UI 输入；UI 不解释 Syncthing 原始状态字符串。

“打开 Web UI”只使用已经通过 loopback 校验的地址，并由 app Effect 调用操作系统能力。UI 不直接启动外部程序。

## 14. 测试策略

### 14.1 基础控件

- Label 的文字、图标、对齐和 DPI 绘制。
- Button 的 hover、pressed、selected、disabled、背景样式、内部释放点击和拖出取消。
- TextBox 的 Plain/Masked 绘制、IME、焦点、提交与剪贴板。
- Masked TextBox 的 DrawList、Action Debug 和日志不包含 API Key 明文。
- SensitiveText 的脱敏 Debug 和销毁清理。
- Switch 与 Checkbox 的鼠标、Space 键、焦点和 Toggle Action。
- 亮色与暗色 Theme Token 下的控件前景、背景、边框和焦点环选择。

### 14.2 容器与 Overlay

- FormRow 的宽屏左右布局和窄屏上下布局。
- FormView 的滚动范围、裁剪、分类切换归零和命中坐标转换。
- Modal Overlay 中所有鼠标、滚轮、键盘和 IME 事件不下穿。
- Escape 和关闭按钮关闭浮层；遮罩点击不关闭。
- 关闭后恢复原焦点和 IME。
- 小窗口和不同 DPI 下 ModalFrame 不越界。
- CategoryNavigation 选中背景、FormSection 圆角 Surface 和内部行分隔线符合语义 Token。

### 14.3 app 边界与持久化

- SettingsViewInput 只包含纯数据。
- ControlAction 到 SettingsViewAction 的穷尽映射。
- 设置即时生效、原子持久化、重启后恢复和保存失败重试。
- `ui` 公共边界测试确保不依赖 app、DocumentView 或 Syncthing DTO。

### 14.4 Syncthing 设置

- API Key 首次配置、更新、空提交和不回显。
- loopback 地址允许/拒绝矩阵。
- 连接测试的 Testing、成功、认证失败、不可用和版本不支持状态。
- Device ID 复制、打开 Web UI和断开控制连接。
- 断开操作不修改 Syncthing 设备、资料库、进程或用户文件。

## 15. 分阶段实施

本功能跨越多个模块，按照项目规范拆成独立子任务；每阶段完成后先编译和测试，再进入下一阶段。

1. 基础控件：Label、Button 扩充、TextBox Widget 化、Switch、Checkbox、统一 Action。
2. 通用容器：InlineGroup、FormRow、FormSection、FormView。
3. Overlay 抽象：扩充现有 UiShell Overlay 的布局、输入和关闭策略。
4. 设置业务视图：四个分类、真实设置、即时生效、持久化和失败重试。
5. Syncthing 设置接入：纯数据输入、异步状态映射、安全密钥提交和控制动作。

每阶段执行 `cargo fmt`、对应 crate 测试和编译。全部完成后执行重大修改验证命令 `./scripts/verify.sh`。

## 16. 验收标准

- 用户可从应用内打开唯一的设置浮层，背景编辑器可见但完全不可交互。
- 设置界面呈现统一的 macOS System Settings 风格：侧栏选中块、清晰标题层级、分组 Surface、克制分隔线和紧凑系统控件比例。
- 设置页由通用 Overlay、Form 容器和基础控件组合，不存在 Syncthing 专用 UI 控件。
- 四个分类中的真实设置可校验、即时生效、持久化并在重启后恢复。
- API Key 可首次配置和更新，但不会被 UI 回显、DrawList、Debug 或日志泄露。
- 连接测试可展示本机 Device ID、Syncthing 版本和明确错误状态。
- 用户可复制 Device ID、打开本机 Web UI并安全断开 Textora 控制连接。
- Overlay 关闭后恢复原编辑器焦点和 IME 状态。
- `ui` 与 `app`、Syncthing 领域保持纯数据输入和语义 Action 边界。
