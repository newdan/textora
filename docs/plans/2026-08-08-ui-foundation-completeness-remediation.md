# 基础 UI 控件完备性修复计划

> 本计划用于补齐 `textora-ui` 基础控件在键盘焦点、动作契约、交互取消、状态同步、可访问性和极端布局方面的系统性缺口。执行时逐项勾选；每个行为修复必须先写失败测试，再修改实现。

**目标：** 让 Button、List、PopupMenu、Checkbox、Switch、Splitter、Tooltip 等基础控件既能在当前产品页面中工作，也能通过统一协议独立复用；鼠标、键盘、IME、焦点和可访问性状态必须共享同一套可验证契约。

**架构：** `ui` 层继续只接收纯数据与产品无关事件，由叶子控件声明身份、焦点能力、交互状态和语义节点；容器负责焦点作用域、坐标变换和事件所有权；`appkit-shell` 负责把 winit 窗口事件与平台可访问性事件适配为共享协议；`app` 只翻译业务 action，不复制控件内部命中逻辑。

**范围：** 本计划修复共享 UI 基础设施和 textora 中直接使用这些控件的组合路径，不修改文档模型、编辑算法、插件协议、持久化格式或产品业务规则。

## 1. 当前基线

2026-08-08 审查确认：

- `cargo test -p textora-ui` 通过：911 个单元测试、6 个集成测试通过，1 个文档测试忽略；
- `ui` 与 `app` 的依赖边界已有静态测试保护；
- `WidgetAction`、DPI、局部坐标、裁剪、overlay、指针捕获、IME 和敏感文本基础已经存在；
- `TextBox` 的 grapheme、选择、IME、剪贴板和敏感文本覆盖较完整；
- 当前缺陷主要是已有协议之间没有闭环，而不是控件文件数量不足。

已确认的缺口：

1. `Button` 有 `WidgetId`，但不进入焦点链，也不支持 Enter/Space；
2. `ListAction::Selected` 存在，但 `ListWidget::on_event` 没有任何路径发出它；
3. `PopupMenuWidget` 只处理鼠标和 Escape，菜单项也没有 disabled 状态；
4. `Event` 没有 pointer leave、capture cancel 或 window focus lost 语义；
5. `Widget` 没有 role、name、value、checked、disabled 等可访问性语义；
6. `Checkbox` 和 `Switch` 的 `checked/enabled` 外部同步 API 缺失；
7. `SplitterWidget` 实现了键盘调整，却没有合法的通用焦点入口；
8. Tooltip 单行宽度不封顶，文本宽于窗口时可能生成负坐标；
9. crate 根级 `allow(dead_code)`、`allow(unused_must_use)` 等抑制会掩盖控件契约未使用和 action 丢失。

## 2. 设计不变量

实现完成后必须持续满足以下不变量：

1. 需要键盘的控件必须拥有稳定 `WidgetId`，并显式声明 `is_focusable`；
2. 一个键盘事件最多到达一个焦点所有者；modal/popup 打开时不得下穿到底层焦点；
3. 可点击控件的鼠标与键盘激活必须产生同一种语义 action；
4. 按下、拖动或 hover 状态都必须能由 release、pointer leave、capture cancel 或 window focus lost 结束；
5. 外部模型状态与短暂交互状态分离：`checked/selected/enabled/value` 可同步，`hovered/pressed/dragging` 由控件内部维护；
6. 容器只能消费叶子控件返回的 typed action，不得调用叶子控件内部 hit-test 复制选择规则；
7. 每个可交互控件都能生成产品无关的可访问性语义；视觉 Label 与控件之间可建立名称或描述关系；
8. 所有尺寸先以逻辑像素定义，只在布局/绘制边界乘一次 DPI；
9. `ui` 不得依赖 `DocumentView`、Workspace、AppAction 或产品状态结构体；
10. 每个实现子任务最多修改 3 个文件，每次提交前至少完成目标 crate 的编译与定向测试。

## 3. 目标协议

### 3.1 焦点作用域

叶子控件只负责：

- 返回自身 `WidgetId` 和是否可聚焦；
- 接收 `set_keyboard_focus`；
- 鼠标按下时返回 `ControlAction::FocusRequested`；
- 仅在自身已聚焦时处理键盘激活。

容器负责：

- 维护作用域内唯一 `focused_id`；
- Tab/Shift+Tab 遍历 `collect_focusable_ids`；
- 消费并应用子控件的 `FocusRequested`；
- modal 打开时把焦点限制在 modal 作用域，关闭后恢复原焦点；
- 不把未处理键盘事件广播给所有子控件。

### 3.2 控件状态

统一把状态分成两类：

```text
外部可同步状态：enabled / checked / selected / value / items
内部短暂状态：hovered / pressed / dragging / focused / preedit
```

外部状态通过 `set_input` 或精准 setter 更新。若控件正在捕获指针，输入同步不得无意中覆盖本次交互的临时位置；当控件变为 disabled 时必须立即清除 hover、pressed、dragging 和 focus。

### 3.3 Action 语义

- Button：鼠标释放、Enter、Space 都产生 `ControlAction::Activated`；
- Checkbox/Switch：鼠标释放、Space 都产生 `ControlAction::Toggled`；
- List：成功的 press/release 产生 `ListAction::Selected`，关闭按钮产生 `CloseRequested`；
- PopupMenu：鼠标选择与键盘 Enter 产生同一个 `PopupOutcome::Selected`；
- Splitter：指针拖动和键盘调整都产生 `SplitterAction`，但只有配置了 ID 的实例进入焦点链；
- 所有 action 都由最近的组合容器翻译一次，禁止上层重新命中计算。

### 3.4 可访问性语义

在 `ui` 内定义平台无关的最小语义模型：

- `AccessibilityRole`：Button、Checkbox、Switch、TextField、List、ListItem、Menu、MenuItem、Separator、Slider、Dialog、StaticText；
- `AccessibilityNode`：稳定 ID、role、name、description、value、bounds、focused、disabled、checked/selected 状态与可用 action；
- `AccessibilityAction`：Focus、Activate、Toggle、SetValue、Increment、Decrement、Dismiss；
- `Widget` 提供收集节点和处理语义 action 的入口；
- `appkit-shell` 将语义树和 action 适配到平台后端，`app` 不解释具体 role。

在选择平台后端前，先以纯 Rust 语义树测试固定协议；后端依赖版本和平台支持情况单独验证，不把平台类型泄漏进 `ui`。

## 4. 分阶段实施

---

### Phase 1：修复叶子控件焦点与状态契约

#### Task 1.1：Button 键盘焦点闭环

**文件：**

- Modify: `crates/ui/src/widgets/button.rs`
- Modify: `crates/ui/src/widgets/modal_frame.rs`
- Modify: `crates/ui/src/widgets/settings_view/widget.rs`

**步骤：**

- [ ] 先增加失败测试：Button 可被 `collect_focusable_ids` 收集；
- [ ] 增加失败测试：未聚焦时 Enter/Space 不激活，聚焦后两者均激活；
- [ ] 增加失败测试：鼠标按下请求焦点，但仍保留 press/release 激活语义；
- [ ] 为 Button 增加 `focused` 状态、焦点环绘制和 disabled 时的状态清理；
- [ ] 让 ModalFrame 的关闭按钮进入 modal 焦点顺序，并能通过 Enter/Space 关闭；
- [ ] 让 SettingsView 的分类按钮和表单内分段按钮使用同一焦点请求协议；
- [ ] 验证设置页分段按钮及 modal 关闭按钮没有鼠标行为回归。

**定向验证：**

```bash
cargo test -p textora-ui widgets::button
cargo test -p textora-ui widgets::inline_group
cargo test -p textora-ui widgets::modal_frame
```

#### Task 1.2：Checkbox 与 Switch 外部状态同步

**文件：**

- Modify: `crates/ui/src/widgets/checkbox.rs`
- Modify: `crates/ui/src/widgets/switch.rs`
- Modify: `crates/ui/tests/public_api.rs`

**步骤：**

- [ ] 先增加外部 crate 视角的 API 编译测试；
- [ ] 增加 `checked()`、`set_checked()`、`is_enabled()`、`set_enabled()`；
- [ ] `set_enabled(false)` 必须清除 hover、pressed 和 focus；
- [ ] 外部 `set_checked` 不产生用户 action；
- [ ] 用户 toggle 后返回新值，随后同值同步不得反向翻转；
- [ ] disabled 控件不进入焦点链、不请求焦点、不响应键盘或鼠标。

**定向验证：**

```bash
cargo test -p textora-ui widgets::checkbox
cargo test -p textora-ui widgets::switch
cargo test -p textora-ui --test public_api
```

#### Task 1.3：Splitter 的焦点能力改为显式可选

**文件：**

- Modify: `crates/ui/src/widgets/splitter.rs`
- Modify: `crates/ui/tests/public_api.rs`

**步骤：**

- [ ] 先增加失败测试：默认 Splitter 保持 pointer-only；
- [ ] 增加 `with_id` 或等价的显式可聚焦构造方式；
- [ ] 仅带 ID 且 enabled 的 Splitter 进入焦点链；
- [ ] Left/Right/Home/End 仅在聚焦时生效；
- [ ] 增加焦点环或明确的 focused 视觉，不复用 hover 状态；
- [ ] 验证当前未配置 ID 的产品 splitter 不再响应误路由键盘。

---

### Phase 2：关闭 List 与 PopupMenu 的动作断层

#### Task 2.1：ListWidget 独立选择协议

**文件：**

- Modify: `crates/ui/src/widgets/list.rs`
- Modify: `crates/ui/src/widgets/sidebar/mod.rs`
- Modify: `crates/ui/src/widgets/sidebar/widget_tests.rs`

**步骤：**

- [ ] 先增加失败测试：普通行 press/release 返回 `ListAction::Selected(index)`；
- [ ] 增加 press row 状态，释放到其他行或列表外时取消选择；
- [ ] close button 与 row selection 互斥，一次点击只能产生一个 action；
- [ ] Separator/Header 永远不能进入 press 或 selection；
- [ ] Sidebar 改为翻译 `WidgetAction::List`，删除对 `hit_row` 的业务性直接调用；
- [ ] 保留 hover、close、固定标签和滚动偏移的现有行为。

**定向验证：**

```bash
cargo test -p textora-ui widgets::list
cargo test -p textora-ui widgets::sidebar
```

#### Task 2.2：ListWidget 可选键盘导航

**文件：**

- Modify: `crates/ui/src/widgets/list.rs`
- Modify: `crates/ui/tests/public_api.rs`

**步骤：**

- [ ] 为 List 增加可选 ID 和 focused row；
- [ ] Up/Down 或 Left/Right 按 orientation 移动焦点，跳过 Header/Separator；
- [ ] Home/End 移到首尾可选项，Enter/Space 产生 Selected；
- [ ] 输入列表变化后，以稳定规则保留或规范化 focused row；
- [ ] 无 ID 的视觉列表保持 pointer-only，避免破坏现有组合控件。

#### Task 2.3：PopupMenu 键盘与 disabled 状态

**文件：**

- Modify: `crates/ui/src/widgets/popup_menu/types.rs`
- Modify: `crates/ui/src/widgets/popup_menu/mod.rs`
- Modify: `crates/appkit-shell/src/ui_shell.rs`

**步骤：**

- [ ] 为 `PopupMenuItem` 增加 enabled 状态，并以构造 helper 避免调用方重复字段；
- [ ] 菜单打开时选择首个 enabled 非分隔项，或保留显式初始项；
- [ ] Up/Down/Home/End 跳过 disabled 与 separator；
- [ ] Enter/Space 选择，Escape 关闭；disabled 项的鼠标和键盘均不产生 action；
- [ ] popup 打开期间消费未识别键盘事件，禁止下穿底层焦点；
- [ ] UiShell 在 popup 打开/关闭时保存并恢复焦点所有权；
- [ ] 增加全 disabled、首尾分隔、单项和超长菜单测试。

---

### Phase 3：补齐 Pointer Leave 与 Interaction Cancel

#### Task 3.1：扩展共享事件协议

**文件：**

- Modify: `crates/ui/src/core/widget.rs`
- Modify: `crates/ui/src/core/dock.rs`
- Modify: `crates/ui/src/widgets/inline_group.rs`

**步骤：**

- [ ] 先为 hover 残留和捕获残留增加失败测试；
- [ ] 增加产品无关的 `PointerLeave` 与 `InteractionCancel` 事件；
- [ ] `PointerLeave` 广播给可见 hover owner，但不结束合法的 pointer capture；
- [ ] `InteractionCancel` 必须结束所有 pressed/dragging/capturing 状态；
- [ ] Dock 和 InlineGroup 只负责传播，不创建控件特定 action；
- [ ] 更新 `Event::Debug`、zeroize 和坐标变换的穷尽匹配。

#### Task 3.2：叶子控件清理瞬态状态

本任务按每组最多 3 个文件继续拆分：

- [ ] 组 A：Button、Checkbox、Switch；
- [ ] 组 B：Scrollbar、Splitter、CanvasScrollbars；
- [ ] 组 C：List、PopupMenu、Tooltip owner；
- [ ] 组 D：TextBox、TagEditor、SearchBar。

每组都必须先覆盖：hover leave、press 后 cancel、drag 后 cancel、disabled during capture，以及重复 cancel 的幂等性。

#### Task 3.3：接入 winit 生命周期

**文件：**

- Modify: `crates/app/src/app_lifecycle.rs`
- Modify: `crates/app/src/events.rs`
- Modify: `crates/appkit-shell/src/ui_shell.rs`

**步骤：**

- [ ] `WindowEvent::CursorLeft` 转为 `PointerLeave`；
- [ ] `WindowEvent::Focused(false)` 转为 `InteractionCancel`；
- [ ] UiShell 将 cancel 发送给 overlay、canvas scrollbar、dock 和当前捕获 owner；
- [ ] 失焦时清除 tooltip timer/overlay 与系统 cursor hint；
- [ ] 增加 Button press、Scrollbar drag、Splitter drag 和 modal press 的窗口失焦回归测试；
- [ ] 确认 editor runtime 与产品 UI 的 cancel 各自执行一次，不重复产生业务 action。

---

### Phase 4：建立可访问性语义树

#### Task 4.1：定义平台无关语义模型

**文件：**

- Add: `crates/ui/src/core/accessibility.rs`
- Modify: `crates/ui/src/core/mod.rs`
- Modify: `crates/ui/src/core/widget.rs`

**步骤：**

- [ ] 定义 role、state、action、node 和稳定语义 ID；
- [ ] `Widget` 默认不产生节点，但可递归收集子节点；
- [ ] bounds 使用屏幕物理像素，容器偏移只能应用一次；
- [ ] 敏感 TextBox 不得把真实 value 放入语义树；
- [ ] 对重复 ID、无效 bounds 和孤立 focused node 提供验证错误，而不是静默接受。

#### Task 4.2：覆盖叶子基础控件

按每组最多 3 个文件实施：

- [ ] Button、Checkbox、Switch；
- [ ] TextBox、Label、Tooltip；
- [ ] Splitter、Scrollbar、CanvasScrollbars；
- [ ] List、TreeList、VirtualCardList；
- [ ] PopupMenu、SplitButton、EditorToolbar。

每个控件至少验证 role、name/value、focused、disabled、checked/selected、bounds 与可用 action；所有语义 action 必须复用现有 typed action，不得开第二条业务通道。

#### Task 4.3：覆盖容器与 modal 焦点边界

**文件：**

- Modify: `crates/ui/src/core/dock.rs`
- Modify: `crates/ui/src/widgets/form/row.rs`
- Modify: `crates/ui/src/widgets/modal_frame.rs`

**步骤：**

- [ ] Dock 按视觉层级构造语义子树；
- [ ] FormRow 将 Label/description 关联到 control；
- [ ] modal 节点声明 Dialog role，并只暴露最上层 modal 子树；
- [ ] 隐藏、裁剪出视口或 disabled 的节点遵循统一暴露规则；
- [ ] 焦点恢复后语义树中只能有一个 focused node。

#### Task 4.4：平台后端接入

**文件：**

- Modify: `crates/appkit-shell/Cargo.toml`
- Add: `crates/appkit-shell/src/accessibility_adapter.rs`
- Modify: `crates/appkit-shell/src/lib.rs`

**步骤：**

- [ ] 先确认当前 winit 版本可用的平台可访问性后端及版本兼容性；
- [ ] 将纯语义树转换为平台节点树；
- [ ] 将平台 Focus/Activate/Toggle/SetValue 等 action 转回 `WidgetAction`；
- [ ] 后端不可用时允许显式 no-op adapter，但不得影响 UI 行为；
- [ ] 至少完成 macOS VoiceOver 的真实窗口手工回归，Windows/Linux 支持状态如实记录。

---

### Phase 5：布局、主题与 API 一致性

#### Task 5.1：Tooltip 长文本与小窗口安全

**文件：**

- Modify: `crates/ui/src/widgets/tooltip.rs`
- Modify: `crates/ui/src/core/text_layout.rs`

**步骤：**

- [ ] 先增加文本宽于窗口、窗口小于 tooltip 最小高度的失败测试；
- [ ] 为 Tooltip 定义最大逻辑宽度和屏幕 margin；
- [ ] 支持按 grapheme/词边界换行，无法换行时安全截断；
- [ ] 宽高始终限制在屏幕有效区域，x/y 不得为负或非有限值；
- [ ] 中英文、emoji、高 DPI 和零尺寸屏幕输入均有覆盖。

#### Task 5.2：基础控件尺寸与视觉 token 收敛

本任务先写一份 token 映射表，再按最多 3 个文件一组迁移：

- [ ] 明确 control height、hit target、corner radius、focus ring、spacing、font size 的语义 token；
- [ ] Button/List 的外部 style 允许覆盖，但默认值必须来自主题/共享 token；
- [ ] Checkbox/Switch 中固定白色和 hover blend 迁移为语义主题色；
- [ ] List close icon 的硬编码灰色迁移到主题；
- [ ] 不在一次提交中全局机械替换；每组迁移都做 light/dark 和 DPI 绘制命令测试。

#### Task 5.3：公共 API 契约测试

**文件：**

- Modify: `crates/ui/tests/public_api.rs`
- Modify: `crates/ui/tests/public_boundaries.rs`
- Modify: `crates/ui/src/lib.rs`

**步骤：**

- [ ] 从外部 crate 视角构造全部基础控件和输入类型；
- [ ] 验证 Widget、action、状态同步和 accessibility 类型的稳定根级路径；
- [ ] 保持 `widgets` 实现模块私有，继续只暴露语义模块；
- [ ] 增加禁止 UI 依赖 app 产品类型和平台可访问性实现类型的边界测试。

---

### Phase 6：质量门禁收口

#### Task 6.1：移除高风险 crate 级 lint 抑制

按 lint 类型逐项处理，不允许一次性删除后堆积无关修改：

- [ ] 优先移除 `allow(unused_must_use)`，修复所有被丢弃的 Result/action；
- [ ] 移除 `allow(dead_code)`，删除死代码或把真正的公共能力纳入契约测试；
- [ ] 移除 `allow(unused_mut)`；
- [ ] 将确有理由的复杂类型、参数数量抑制下沉到最小作用域并写明原因；
- [ ] 禁止新增 crate 级 blanket allow。

#### Task 6.2：最终回归矩阵

| 场景 | 鼠标 | 键盘 | 焦点 | 可访问性 | Cancel |
|---|---|---|---|---|---|
| Button | release 激活 | Enter/Space 激活 | 可 Tab、可见焦点环 | Button/Activate | 清除 pressed |
| Checkbox/Switch | release toggle | Space toggle | disabled 不可聚焦 | checked/disabled | 清除 pressed |
| List | 行选择/关闭互斥 | 方向键 + Enter | 可选焦点入口 | List/ListItem | 取消 press |
| PopupMenu | enabled 项选择 | 方向键 + Enter/Escape | popup 内封闭 | Menu/MenuItem | 关闭或复位 |
| Splitter | 拖动 | 聚焦后方向键 | ID 显式启用 | Slider 增减 | 结束拖动 |
| TextBox | 光标与选择 | 编辑/提交 | 唯一文本焦点 | TextField/SetValue | 清除 drag/preedit |
| Tooltip | 不拦截 | 不适用 | 不可聚焦 | description | leave 隐藏 |
| Modal | 内部命中 | Tab trap/Escape | 关闭后恢复 | Dialog | 失焦取消 |

## 5. 测试与提交策略

每个任务遵循：

1. 写最小失败测试并确认 RED；
2. 实现根因修复，不叠加防御性 bool；
3. 运行目标模块测试；
4. 运行目标 crate 全量测试和 `cargo check`；
5. `cargo fmt --all -- --check`；
6. 删除死代码、无用注释和未使用 import；
7. 单独提交，提交信息精确描述一个协议变化。

同一缺陷若连续修改超过两次仍未闭环，停止继续补丁，重新审查事件所有权、焦点作用域或状态真相来源。

## 6. 最终验证门槛

全部阶段完成后必须依次通过：

```bash
cargo fmt --all -- --check
cargo test -p textora-ui
cargo test -p textora-appkit-shell
cargo test -p textora-app --lib
cargo check --workspace --all-targets
./scripts/verify.sh
```

并完成真实窗口手工验证：

- [ ] 仅用键盘打开设置页、遍历全部字段、切换选项、触发重试并关闭 modal；
- [ ] 鼠标按下控件后移出窗口或切换应用，返回后无残留 pressed/dragging/tooltip；
- [ ] PopupMenu 可完全用键盘选择，disabled 项不可激活且事件不下穿；
- [ ] 超长中英文 Tooltip 在 1x/2x DPI 和窄窗口内完整受限；
- [ ] macOS VoiceOver 能读取 Button、Switch、Checkbox、TextBox、List、Menu 和 Dialog 的名称、状态与焦点；
- [ ] 关闭 modal 后键盘焦点和可访问性焦点都恢复到打开前的控件。

## 7. 完成定义

只有满足以下全部条件，才可将本计划标记完成：

- 所有基础控件的鼠标、键盘、焦点、状态同步与 cancel 契约均有自动化测试；
- `ListAction::Selected` 等公开 action 都存在可达的真实交互路径；
- 产品组合层不再直接调用叶子控件内部 hit-test 复制动作逻辑；
- modal/popup 输入不下穿，窗口失焦不会留下瞬态交互状态；
- 可访问性语义树已接入至少一个真实平台后端并完成手工验证；
- `ui`/`app` 跨层边界测试继续通过；
- 高风险 crate 级 lint 抑制已移除或缩小到有说明的局部范围；
- 完整 `./scripts/verify.sh` 通过，且手工回归结果记录在本计划或独立验收文档中。
