# Notora 键盘焦点与 Modal 输入所有权执行计划

> 本计划用于修复 Notora 中无焦点控件响应键盘、modal 输入穿透、局部编辑状态与产品焦点不同步，以及标签输入法事件丢失等同源问题。执行时逐项勾选，每个行为变更必须先写失败测试。

**目标：** 建立唯一、可测试的输入所有权协议，使键盘事件只到达当前焦点控件，IME 只到达当前文本焦点，指针捕获只影响鼠标事件，modal 阻断所有底层输入及不允许的应用快捷键。

**架构：** 以 `NotoraState::layout.focus_target` 作为产品键盘焦点的唯一真相，将 `NotoraShell` 的统一事件入口拆成 pointer、keyboard、IME 三条类型化路由。产品 overlay 和编辑器局部 popup 在焦点控件之前取得输入所有权；应用快捷键只在没有 modal 阻断时解析；Editor 焦点下未被产品层消费的键盘与 IME 继续交给 `EditorRuntime`。

**范围：** 本计划只调整 Notora 产品壳、Notora 编辑器 chrome 和标签输入控件的输入协议，不修改 mmap 解析、画布布局、文档编辑算法、catalog 或持久化格式。

**关联计划：** `docs/plans/2026-08-06-notora-input-routing-root-fix.md` 已处理 pointer hover、捕获、画布手势和系统光标。本计划在其基础上补齐 keyboard、IME、focus 和 modal 契约，不回退既有 pointer 双路反馈行为。

## 执行记录（2026-08-07）

代码实现和自动化验证已完成，桌面 GUI 手工回归尚未完成：

- splitter 仅接收指针事件；产品 Overlay、局部 Popup、keyboard/IME 与 pointer 已进入独立私有路由，底层控件不再依靠事后 `consumed` 兜底。
- `EditorRuntime` fallback 显式要求 `FocusTarget::Editor`；其他产品焦点下的未识别按键不会再进入 runtime。
- 标签编辑状态随产品焦点同步，离开标签焦点时保留已提交到 draft、但尚未创建 chip 的文本；TagEditor 支持 preedit、commit、enable/disable 及 IME 光标矩形。
- 审查补充的回归测试覆盖 NewDocumentMenu 对紧凑布局按钮的阻断、局部 Popup 对 canvas scrollbar 的阻断，以及程序化切焦时的标签草稿保留。
- 已通过 Rust 1.93 工具链下的定向测试、`cargo check -p notora-app --all-targets`、`cargo fmt --all -- --check` 与沙箱外完整 `./scripts/verify.sh`。
- 由于当前环境没有可操作的 Notora 应用 bundle，尚未执行第 6 节桌面 GUI 矩阵。因此“自动化完成”，但“包含真实窗口手工验证的最终验收”仍为待办，不再笼统标记 Task 1–6 全部完成。

## 1. 已确认问题

### 1.1 Splitter 绕过焦点

`NotoraShell::route_event_with_context` 在按 `FocusTarget` 分发之前无条件调用 `route_splitter_event`。`SplitterWidget` 会响应 `Left`、`Right`、`Home`、`End`，但 `FocusTarget` 没有 splitter 变体，因此 splitter 的键盘响应没有合法焦点来源。

结果：

- Editor、NavigationSearch、NavigationTree 或 CardList 获得焦点时，方向键仍可能改变 pane 宽度；
- 三栏模式下导航 splitter 总是优先于卡片栏 splitter；
- 导航宽度到达边界后，同一类按键会回退到卡片栏 splitter；
- 错误宽度触发 redraw、响应式布局重算和 session 持久化；
- EditorRuntime、搜索框和导航树无法收到原本属于它们的按键。

### 1.2 全局快捷键绕过 modal 和焦点控件

`events.rs` 在构造并路由 `ui::Event::KeyDown` 之前直接处理 Escape、Cmd/Ctrl+O、Cmd/Ctrl+逗号、Cmd/Ctrl+N、Cmd/Ctrl+F 和 Cmd/Ctrl+S。

结果：

- Settings、NewDocumentMenu、SaveConflict 或确认弹窗打开时，应用快捷键仍可能操作背后页面；
- Escape 无法先交给 EditorTitle、EditorTag 或局部 popup；
- 快捷键可以让 `focus_target` 与仍然打开的 overlay/popup 状态形成不一致组合。

### 1.3 确认弹窗不是严格 modal

Settings、NewDocumentMenu 和 SaveConflict 会无条件消费未处理事件；删除/恢复确认弹窗只有命中按钮或背景时才提前返回。App 最后的 `route.consumed || product_modal_is_open` 只能改变返回值，无法撤销 route 内已经产生的 splitter 或底层 widget action。

结果：

- 确认弹窗打开时方向键仍可能修改 pane；
- 鼠标移动可能更新底层 hover 和 cursor hint；
- 点击确认面板内部空白区域可能继续命中背后控件。

### 1.4 标签编辑状态与产品焦点双轨

`EditorPaneChrome` 使用 `tag_editor_active` 决定标签是否先接收键盘，但 `NotoraShell::synchronize_focus` 不会在焦点离开 `EditorTag` 时清除此状态。

典型路径：

1. 点击标签区域，`tag_editor_active = true`；
2. Cmd/Ctrl+F 将产品焦点切到 `NavigationSearch`；
3. 后续字符仍先进入标签编辑器，搜索框得不到输入。

### 1.5 标签 IME 协议缺失

`TagEditorWidget` 只处理裸字符、Backspace、Enter 和 Escape，不处理 `ImePreedit`、`ImeCommit`、`ImeEnable`、`ImeDisable`。父级在标签 active 时又会消费未处理事件，同时 `EditorTag` 不提供 IME cursor rect。

结果：

- 中文等输入法的 preedit 和 commit 可能被静默丢弃；
- 系统候选窗口没有可靠锚点；
- 标签文本编辑与 Search/EditorTitle 使用的 `TextBox` 输入协议不一致。

### 1.6 局部 popup 存在潜在穿透

LocationPicker 和 EditorToolbar overflow 当前产品模型基本未打开，但它们可见时只处理少数事件，未处理键盘会继续落入 splitter、焦点 widget 或 EditorRuntime。启用这些功能前必须纳入统一 popup 输入策略。

## 2. 目标输入协议

### 2.1 所有权优先级

不同事件类型使用不同所有权链，禁止用一条广播式路由同时处理所有输入。

#### Pointer / Wheel

```text
产品 modal
  → 局部 modal popup
  → 当前 pointer capture owner
  → 视觉层级 hit test
  → EditorRuntime（编辑器区域或 editor capture）
```

约束：

- capture 只影响 `MouseMove` 和 `MouseUp` 等指针事件；
- splitter 仅进入 pointer 路由；
- `MouseMove` 可以为清理 hover 而通知多个视觉控件，但不得借此传播键盘或 IME；
- modal 可选择在自身内部广播 hover，但绝不能触达底层页面。

#### Keyboard

```text
产品 modal
  → 局部 modal popup
  → 当前 FocusTarget
  → 允许的应用快捷键 fallback
  → EditorRuntime（仅 FocusTarget::Editor）
```

Escape 使用专门规则：

```text
产品 modal
  → 局部 popup
  → 当前焦点控件
  → NotoraAction::EscapePressed fallback
```

应用快捷键规则：

- modal 打开时默认全部禁止；
- 当前没有必须在 modal 内执行的 Notora 应用快捷键；
- 非 modal 状态下，Cmd/Ctrl+O、逗号、N、F、S 可以在焦点控件之前作为显式应用命令解析；
- Escape 不属于抢占式应用快捷键，必须先给局部所有者处理。

#### IME

```text
产品 modal 内的文本焦点
  → 当前产品文本焦点
  → EditorRuntime（仅 FocusTarget::Editor）
```

约束：

- IME 不得广播；
- NavigationTree、CardList、splitter 等非文本焦点必须返回 ignored；
- `FocusTarget::Overlay` 必须由活动 overlay 的内部文本焦点解释；没有匹配目标时消费并停止；
- `EditorTag`、`EditorTitle`、`NavigationSearch` 必须提供一致的 preedit、commit 和 cursor rect。

### 2.2 状态不变量

每次输入处理完成后必须满足：

1. `focus_target` 是唯一产品键盘焦点；
2. `overlay != OverlayState::None` 时，`focus_target == FocusTarget::Overlay`；
3. `focus_target != FocusTarget::EditorTag` 时，标签编辑器不得继续处于 editing 状态；
4. 一次键盘或 IME 事件最多只有一个 sink；
5. modal 存在时，不得产生底层文档、pane、导航、卡片或应用快捷键 action；
6. pointer capture 不得改变 keyboard/IME sink；
7. `NotoraEventRoute::consumed` 必须表示事件已经被某个所有者接收，不能在副作用发生后用 modal bool 事后伪造。

### 2.3 Widget 契约

- `ui` widget 保持产品无关，不读取 `NotoraState` 或 `FocusTarget`；
- 叶子 widget 可以保留自身 focused/editing 防线，但产品 router 是唯一权威分发者；
- 需要键盘的 widget 必须有合法焦点入口；
- 没有焦点入口的 widget 只能接收 pointer 事件；
- modal/popup 对未识别事件也必须返回 consumed，不能依靠具体子控件恰好消费。

## 3. 非目标

- 本阶段不为 splitter 引入 Tab 焦点、无障碍语义或焦点环；
- 本阶段不改变 splitter 的鼠标拖动体验、宽度范围和持久化格式；
- 本阶段不修改 mmap 节点方向键语义；只保证按键能到达 EditorRuntime；
- 本阶段不重构全部 `ui::Widget` 为自持有全局焦点；
- 本阶段不改变 Search、NavigationTree、CardList 的产品 action；
- 本阶段不改变 overlay 的视觉样式或业务决策。

## 4. 文件责任边界

| 文件 | 计划后的责任 |
|---|---|
| `crates/notora-app/src/events.rs` | winit 输入归一化、modal/快捷键优先级、产品与 EditorRuntime 的顶层组合 |
| `crates/notora-app/src/app.rs` | 应用 route 结果、同步焦点、调用 runtime，不复制 widget 命中规则 |
| `crates/notora-app/src/render.rs` | NotoraShell 的 pointer/keyboard/IME 类型化产品路由、overlay 输入策略 |
| `crates/notora-app/src/editor_pane.rs` | EditorTitle、EditorTag、局部 popup 的焦点内路由 |
| `crates/notora-app/src/state.rs` | 唯一产品 `FocusTarget`、overlay/focus 状态不变量 |
| `crates/ui/src/widgets/tag_editor.rs` | 产品无关的标签文本、preedit、commit、IME cursor 数据 |

## 5. 分阶段执行

每个实现子任务最多修改 3 个文件。若实际实现需要触及第 4 个文件，必须停止并继续拆分，不得扩大当前提交。

---

### Task 1：固定当前缺陷的失败测试

**文件：**

- Modify: `crates/notora-app/src/render.rs`
- Modify: `crates/notora-app/src/app.rs`
- Modify: `crates/notora-app/src/editor_pane.rs`

#### Step 1：补 splitter 焦点隔离测试

- [ ] 在 `render.rs` 测试模块创建完成布局的 `NotoraShell`；
- [ ] 以 `FocusTarget::Editor` 路由 Left、Right、Home、End；
- [ ] 断言 `route.consumed == false`；
- [ ] 断言 `route.actions` 不包含 `NotoraAction::SplitterDragged`；
- [ ] 对 `NavigationSearch` 和 `NavigationTree` 重复验证，确认按键由对应 widget 接收而不是 splitter。

建议测试名：

```rust
editor_keyboard_navigation_never_reaches_unfocused_splitters
navigation_keys_follow_the_product_focus_target
```

#### Step 2：补 modal 无副作用测试

- [ ] 打开删除或恢复确认 overlay；
- [ ] 记录导航宽度、卡片栏宽度、活动文档摘要和 pending session persist 状态；
- [ ] 路由 Left、Right、普通字符和面板空白区 MouseDown；
- [ ] 断言事件被消费；
- [ ] 断言宽度、文档、导航和持久化 deadline 均不变化。

建议测试名：

```rust
confirmation_overlay_blocks_all_underlying_input
modal_keyboard_input_cannot_schedule_layout_persistence
```

#### Step 3：补标签焦点切换测试

- [ ] 激活标签编辑器并输入一个字符；
- [ ] 将产品焦点切到 `NavigationSearch`；
- [ ] 再输入字符；
- [ ] 断言标签 draft 不再变化，字符进入搜索框；
- [ ] 路由 Escape，断言当前焦点所有者先获得处理机会。

建议测试名：

```rust
leaving_editor_tag_focus_stops_tag_keyboard_capture
focused_control_handles_escape_before_product_fallback
```

#### Step 4：验证 RED

- [ ] 运行：

```bash
cargo test -p notora-app --lib render::tests
cargo test -p notora-app --lib editor_pane::tests
cargo test -p notora-app --lib app::tests
```

预期：新增测试因 splitter 抢键、确认 overlay 穿透和 tag active 未清理而失败；不得通过放宽断言把测试改绿。

#### Step 5：提交测试基线

- [ ] 仅在团队允许提交 RED 测试时单独提交；否则保留为下一任务同提交中的第一部分。

---

### Task 2：拆分 NotoraShell 的事件类型路由

**文件：**

- Modify: `crates/notora-app/src/render.rs`
- Modify: `crates/notora-app/src/state.rs`

#### Step 1：建立类型化私有入口

- [ ] 保留公开 `route_event` 作为薄分派层；
- [ ] 新增：

```rust
fn route_pointer_event_with_context(...)
fn route_keyboard_event_with_context(...)
fn route_ime_event_with_context(...)
```

- [ ] 使用 `match event` 一次完成事件分类；
- [ ] 禁止 keyboard/IME 入口调用 pointer hit-test 或 splitter；
- [ ] 禁止 pointer 入口根据 `focus_target` 广播键盘状态。

#### Step 2：收紧 splitter 路由

- [ ] 将 `route_splitter_event` 限定为 `MouseMove`、`MouseDown`、`MouseUp`；
- [ ] capture 中的 splitter 继续收到移出矩形后的 MouseMove/MouseUp；
- [ ] Wheel、KeyDown、IME 一律不进入 splitter；
- [ ] 保持 navigation splitter 与 card-list splitter 的指针层级不变。

#### Step 3：按 FocusTarget 路由键盘

- [ ] `NavigationSearch` → `search_box`；
- [ ] `NavigationTree` → `navigation_tree`；
- [ ] `CardList` → `card_list`；
- [ ] `EditorTitle` / `EditorTag` → `editor_pane` 对应子目标；
- [ ] `Editor` → ignored，由 app 继续交给 EditorRuntime；
- [ ] `Overlay` → 只能由活动 overlay 处理；无匹配 overlay 时 consumed，防止穿透。

#### Step 4：让 modal 在 Shell 内真正短路

- [ ] 为 Settings、NewDocumentMenu、SaveConflict、Trash confirmation 建立统一的 modal 判定；
- [ ] modal route 无 action 时仍返回 consumed；
- [ ] 删除 `product_modal_is_open` 对底层副作用的事后兜底依赖；
- [ ] 保留它作为 debug invariant 或在后续任务完全移除，但不得继续承担输入阻断职责。

#### Step 5：验证 GREEN

- [ ] 运行：

```bash
cargo test -p notora-app --lib render::tests
cargo test -p notora-app --lib state::tests
cargo check -p notora-app
cargo fmt --all -- --check
```

预期：Task 1 中 splitter 与 confirmation overlay 测试通过；已有 pointer、canvas scrollbar 和响应式布局测试保持通过。

#### Step 6：提交

- [ ] 建议提交信息：

```text
fix(notora): route keyboard input by product focus
```

---

### Task 3：修正窗口键盘与快捷键优先级

**文件：**

- Modify: `crates/notora-app/src/events.rs`
- Modify: `crates/notora-app/src/app.rs`

#### Step 1：补快捷键 modal 测试

- [ ] 分别在 Settings、NewDocumentMenu、SaveConflict 和 Trash confirmation 下测试 Cmd/Ctrl+N、O、F、S；
- [ ] 断言 overlay 类型、focus_target、活动文档和外部请求状态不变化；
- [ ] 测试 Escape 仍能关闭产品 overlay；
- [ ] 测试非 modal 状态下现有快捷键行为不变。

建议测试名：

```rust
modal_state_blocks_workspace_shortcuts
escape_closes_modal_before_focused_content
workspace_shortcuts_remain_available_without_a_modal
```

#### Step 2：统一构造 KeyDown

- [ ] winit `KeyboardInput` 首先完成 key code 和 modifiers 归一化；
- [ ] 不再在构造 `ui::Event::KeyDown` 之前直接 return；
- [ ] 新增纯函数描述快捷键意图，例如：

```rust
enum NotoraShortcut {
    OpenExternalFile,
    OpenSettings,
    OpenNewDocumentMenu,
    FocusSearch,
    Save,
}
```

- [ ] 快捷键解析不得直接读取或修改 widget 状态。

#### Step 3：实现优先级

- [ ] modal 打开时先调用产品 modal route；
- [ ] modal 未显式处理的键也必须停止，不解析 workspace shortcut；
- [ ] 非 modal 的 Escape 先路由当前局部 popup/焦点控件；
- [ ] 焦点控件未消费 Escape 时再 dispatch `EscapePressed`；
- [ ] 非 modal 的应用快捷键按既有行为 dispatch；
- [ ] 普通键先走产品焦点，未消费且焦点为 Editor 时交给 runtime。

#### Step 4：消除焦点与 overlay 不一致入口

- [ ] `OpenSettings`、`OpenNewDocumentMenu`、SaveConflict 和确认 overlay 进入时保持 `FocusTarget::Overlay`；
- [ ] modal 关闭后使用明确恢复策略，不依赖快捷键中途写入的焦点；
- [ ] 增加 debug assertion 覆盖 `overlay != None => focus_target == Overlay`；
- [ ] 不新增多个 bool 表示同一互斥状态。

#### Step 5：验证与提交

- [ ] 运行：

```bash
cargo test -p notora-app --lib events::tests
cargo test -p notora-app --lib app::tests
cargo check -p notora-app
cargo fmt --all -- --check
```

- [ ] 建议提交信息：

```text
fix(notora): enforce modal shortcut precedence
```

---

### Task 4：统一 EditorPane 焦点路由

**文件：**

- Modify: `crates/notora-app/src/editor_pane.rs`
- Modify: `crates/notora-app/src/render.rs`

#### Step 1：让 EditorPane 接收显式焦点

- [ ] 将 `EditorPaneChrome::route_event` 改为接收产品焦点或更小的 editor-pane 焦点枚举；
- [ ] 如果引入局部枚举，使用互斥 enum，不使用 `title_focused`、`tag_focused` 多 bool 组合；
- [ ] pointer 仍按矩形命中；
- [ ] keyboard/IME 只按显式焦点进入对应子控件。

推荐局部类型：

```rust
enum EditorPaneFocus {
    Body,
    Title,
    Tag,
    None,
}
```

它只能由 `FocusTarget` 映射生成，不得成为第二份产品焦点状态。

#### Step 2：同步 tag editing 生命周期

- [ ] 新增 `synchronize_focus` 或 `set_focus` 方法；
- [ ] 焦点进入 Tag 时启用 editing；
- [ ] 焦点离开 Tag 时关闭 editing 和 suggestions；
- [ ] 鼠标点击 body 时先结束标签状态，再将同一点击透传给 editor；
- [ ] 文档 key 变化继续清理 draft；
- [ ] Cmd/Ctrl+F、CardActivated、Overlay 打开等程序化切焦必须走同一同步入口。

#### Step 3：限定 toolbar 与 property popup

- [ ] toolbar 在 overflow 未打开时不接收任何 keyboard/IME；
- [ ] LocationPicker、toolbar overflow 和 tag suggestions 可见时声明明确的局部 modal 策略；
- [ ] 局部 modal 对未识别键也返回 consumed；
- [ ] 点击外部关闭 popup 后，是否透传同一点击由单一 helper 明确决定；
- [ ] 禁止 popup 可见性与产品 overlay 争夺同一次键盘事件。

#### Step 4：验证与提交

- [ ] 运行：

```bash
cargo test -p notora-app --lib editor_pane::tests
cargo test -p notora-app --lib render::tests
cargo check -p notora-app
cargo fmt --all -- --check
```

- [ ] 建议提交信息：

```text
fix(notora): synchronize editor chrome focus ownership
```

---

### Task 5：补齐 TagEditor IME 文本协议

**文件：**

- Modify: `crates/ui/src/widgets/tag_editor.rs`
- Modify: `crates/notora-app/src/editor_pane.rs`
- Modify: `crates/notora-app/src/render.rs`

#### Step 1：先写 IME 失败测试

- [ ] preedit 不应直接提交为标签；
- [ ] commit 应在正确字节边界插入 draft；
- [ ] Disable 应清空 preedit，但保留已提交 draft；
- [ ] 非 EditorTag 焦点不得改变 draft；
- [ ] `focused_text_input_ime_cursor_rect` 在 EditorTag 焦点下返回有效矩形；
- [ ] 切走焦点后不再返回标签 IME rect。

建议测试名：

```rust
tag_editor_commits_ime_text_only_while_focused
tag_editor_preedit_has_a_stable_cursor_rect
leaving_tag_focus_clears_preedit_without_committing
```

#### Step 2：选择复用实现

- [ ] 优先在 `TagEditorWidget` 内复用 `TextBox` 的文本、选择、preedit 和 grapheme 边界能力；
- [ ] TagEditor 继续负责 chips、suggestions 和 submit/remove action 映射；
- [ ] 禁止再维护一套按 `char` pop/push 的简化编辑器；
- [ ] 若复用 `TextBox` 会使单函数超过 50 行，拆出 draft-input 子组件或纯 helper。

#### Step 3：暴露 IME cursor rect

- [ ] TagEditor 返回局部 cursor rect；
- [ ] EditorPane 转换为窗口坐标；
- [ ] NotoraShell 在 `FocusTarget::EditorTag` 时返回该 rect；
- [ ] App 继续通过现有 `update_focused_text_input_ime_cursor_area` 设置系统候选窗口位置。

#### Step 4：验证与提交

- [ ] 运行：

```bash
cargo test -p textora-ui tag_editor
cargo test -p notora-app --lib editor_pane::tests
cargo test -p notora-app --lib render::tests
cargo check -p textora-ui
cargo check -p notora-app
cargo fmt --all -- --check
```

- [ ] 建议提交信息：

```text
feat(ui): support focused IME input in tag editor
```

---

### Task 6：补齐端到端输入所有权回归矩阵

**文件：**

- Modify: `crates/notora-app/src/app.rs`
- Modify: `crates/notora-app/src/render.rs`
- Modify: `crates/notora-app/src/events.rs`

#### Step 1：焦点目标矩阵

- [ ] NavigationSearch：Left/Right/Home/End 移动输入光标，IME commit 更新查询；
- [ ] NavigationTree：Left/Right 只折叠或展开节点；
- [ ] CardList：Up/Down/Enter 只选择或打开卡片；
- [ ] Editor：方向键到达 EditorRuntime，pane 宽度不变；
- [ ] EditorTitle：字符、方向键、IME、Escape、Tab 保留标题语义；
- [ ] EditorTag：字符、Backspace、IME、Escape 保留标签语义；
- [ ] Overlay：所有底层状态保持不变。

#### Step 2：响应式布局矩阵

- [ ] ThreePane；
- [ ] NavigationOverlay；
- [ ] EditorOverlay；
- [ ] 每种模式都验证无焦点 splitter 不响应键盘；
- [ ] 每种模式都验证鼠标拖动仅作用于当前可见且 enabled 的 splitter。

#### Step 3：修饰键矩阵

- [ ] 无修饰键 Left/Right；
- [ ] Shift+Left/Right；
- [ ] Alt/Option+Left/Right；
- [ ] Cmd/Ctrl+Left/Right；
- [ ] Home/End；
- [ ] 确认这些组合不会因 splitter 的 `_` 修饰键匹配而泄漏。

#### Step 4：modal 与快捷键矩阵

- [ ] Settings；
- [ ] NewDocumentMenu；
- [ ] SaveConflict；
- [ ] TrashPermanentDeletionConfirmation；
- [ ] TrashRestoreConflictConfirmation；
- [ ] 对每个 modal 验证字符、方向键、Escape、Cmd/Ctrl+N/O/F/S；
- [ ] 除 Escape 的显式关闭语义外，其余事件不得产生底层 action。

#### Step 5：验证与提交

- [ ] 运行：

```bash
cargo test -p notora-app --lib
cargo check -p notora-app --all-targets
cargo fmt --all -- --check
```

- [ ] 建议提交信息：

```text
test(notora): cover keyboard input ownership matrix
```

## 6. 验证门槛

### 每个 Task 提交前

- [ ] `cargo fmt --all -- --check`；
- [ ] 修改 crate 的定向测试通过；
- [ ] `cargo check` 对应 crate 通过；
- [ ] 无 unused import、死代码和临时 debug 输出；
- [ ] 单函数超过 50 行时重新评估并拆分；
- [ ] 不新增宽泛命名和魔法值；
- [ ] 不使用 `.unwrap()`，确定不失败处使用带理由的 `.expect(...)`。

### 全部完成后

- [ ] 切换到仓库要求的 Rust 1.93 工具链；
- [ ] 运行：

```bash
rustup show active-toolchain
cargo fmt --all -- --check
cargo test -p textora-ui tag_editor
cargo test -p notora-app --lib
cargo check -p notora-app --all-targets
./scripts/verify.sh
```

- [ ] 手工验证：

```text
1. 打开 mmap 文档并点击画布；
2. 连续按 Left/Right/Home/End 及其 Shift/Option/Cmd 组合；
3. 确认导航栏和卡片栏宽度不变；
4. 确认编辑器内部导航仍有响应；
5. 点击搜索框、导航树、卡片列表、标题和标签，逐一验证按键归属；
6. 使用中文输入法在搜索、标题、标签和正文输入；
7. 逐个打开产品 modal，确认底层编辑、栏宽和快捷键全部被阻断；
8. 拖动两个 splitter，确认鼠标调宽、光标提示和持久化仍正常；
9. 重启 Notora，确认只有显式拖动产生的宽度被恢复。
```

## 7. 完成定义

只有同时满足以下条件，本计划才算完成：

- [ ] 无焦点 splitter 不响应任何键盘或 IME 事件；
- [ ] 键盘事件只到达当前 `FocusTarget`；
- [ ] IME 只到达当前文本焦点，并具有正确 cursor rect；
- [ ] 所有产品 modal 都在 Shell 内部真正短路底层路由；
- [ ] modal 打开时工作区快捷键不会操作背后页面；
- [ ] Escape 遵循 modal → popup → focused control → product fallback；
- [ ] tag editing 完全由产品焦点派生，不再形成第二焦点源；
- [ ] pointer capture 与 keyboard focus 相互独立；
- [ ] mmap、Markdown、TXT 的 Editor 输入路径一致；
- [ ] 定向测试、workspace 验证和手工回归全部通过；
- [ ] 未改变 catalog、文档格式、session schema 或跨层依赖边界。

## 8. 后续可选工作

以下内容不阻塞本计划：

- 为 splitter 增加 WidgetId、Tab 焦点顺序、焦点环和键盘调宽无障碍能力；
- 将 Notora `FocusTarget` 与 `appkit-shell::KeyboardFocusTarget` 抽象成共享、产品无关的输入所有权接口；
- 为 route outcome 增加仅测试可见的 sink 标识，自动断言一次键盘事件只有一个接收者；
- 将 LocationPicker、toolbar overflow 和 tag suggestions 迁入通用 overlay stack；
- 增加可访问性树中的 modal、focus 和 splitter 语义。
