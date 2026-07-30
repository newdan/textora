# TextBox Widget 设计

## 动机

`SearchBarWidget` 中 find 和 replace 两个文本输入框共享相同的绘制逻辑（背景 rounded rect、边框、文本/placeholder、光标）和键盘处理（InsertChar/Backspace、Enter/Tab 分派），但代码通过 `focus_replace` flag 分支实现，存在重复。随着功能增长（选区、IME、剪贴板），需要抽取一个通用 TextBox 组件。

## 身份

TextBox **不实现 Widget trait**。它是内部组件，嵌入 SearchBarWidget 或其他容器 widget 中使用。Widget trait 不改动。

## 通信

TextBox 通过 struct 上的 callback 字段与父级通信：

```rust
pub on_changed: Option<Box<dyn Fn(&str)>>,       // 文本或光标变更
pub on_enter:   Option<Box<dyn Fn()>>,
pub on_escape:  Option<Box<dyn Fn()>>,
pub on_focus:   Option<Box<dyn Fn(bool)>>,
pub on_copy:    Option<Box<dyn Fn(&str)>>,         // app 层调 clipboard.set
pub on_cut:     Option<Box<dyn Fn(&str)>>,         // app 层调 clipboard.set + 删除文本
pub on_paste:   Option<Box<dyn Fn() -> String>>,   // app 层调 clipboard.get
```

父级（SearchBarWidget 或 app 层代码）在创建 TextBox 时设置回调闭包。

## IME 事件分发

在 `crates/ui/src/core/widget.rs` 的 `Event` 枚举中添加 IME 变体，让 IME 事件通过标准 widget 事件总线流动：

```rust
pub enum Event {
    MouseMove { px: f32, py: f32 },
    MouseDown { px: f32, py: f32, button: MouseButton },
    MouseUp { px: f32, py: f32, button: MouseButton },
    Wheel { dx: f32, dy: f32, px: f32, py: f32 },
    KeyDown(KeyCode),
    // 新增 IME 事件
    ImePreedit { text: String, cursor: Option<(usize, usize)> },
    ImeCommit(String),
    ImeEnable,
    ImeDisable,
}
```

**流转链路**（不破坏 widget 封装层级）：

```
winit Ime::Preedit(text, cursor) → app 包装为 Event::ImePreedit → ui_shell.dispatch(event)
  → dock.dispatch(event) → SearchBarWidget.on_event(event)
    → textbox.on_ime(ImeEvent::Preedit { text, cursor })
```

TextBox 内部定义轻量 IME 事件枚举来接收这些事件：

```rust
#[derive(Clone, Debug)]
pub enum TextBoxIme {
    Preedit { text: String, cursor: Option<(usize, usize)> },
    Commit(String),
    Enabled,
    Disabled,
}
```

**避免直接穿透**：app 层不跨过 ui_shell / SearchBarWidget 直接调 TextBox 方法。

## 内部状态

```rust
pub struct TextBox {
    rect: Rect,

    // 文本状态（自管理）
    text: String,
    cursor_byte: usize,

    // 选区 (anchor_byte, cursor_byte)，不保证 anchor ≤ cursor
    // None = 无选区
    selection: Option<(usize, usize)>,

    // IME（完全内聚）
    preedit: String,
    preedit_cursor: Option<(usize, usize)>,

    // 视觉
    placeholder: String,
    blink_on: bool,
    focused: bool,

    // 鼠标拖选
    dragging: bool,

    // 布局缓存（layout 阶段计算，paint 阶段读取）
    cursor_x: f32,
    preedit_width: f32,
}
```

## IME 封装

TextBox 完全拥有 IME 生命周期：

- **存储**：`preedit`、`preedit_cursor` 存在 TextBox 内部，app 不再持有 `app.preedit_text` 等字段
- **渲染**：`TextBox::paint()` 内绘制 composing text + 下划线标记，不再依赖 app_renderer 外挂 GPU 顶点
- **Commit**：`commit_ime(text)` 拼接文本到 cursor 位置、移动光标、触发 `on_changed`
- **Cursor area**：TextBox 暴露 `ime_cursor_rect() -> Rect`，app 层读取后调用 `window.set_ime_cursor_area()`

### IME 相关方法

```rust
pub fn on_ime(&mut self, ev: &TextBoxIme)
pub fn ime_cursor_rect(&self) -> Rect
pub fn has_preedit(&self) -> bool
```

## 公开方法

### 布局

```rust
/// 在父级布局阶段调用。计算并缓存 cursor_x、preedit_width 等测量值。
/// ctx.ui_measure 提供 UI 字体（proportional）的文本测量能力。
pub fn layout(&mut self, rect: Rect, ctx: &mut LayoutCtx)
```

`layout()` 利用 `ctx.ui_measure`（proportional 字体测量）准确计算文本像素宽度，缓存 cursor_x 供 `paint()` 直接读取。与项目现有 `LayoutCtx` 机制一致。

### 输入

```rust
/// 处理键盘事件。modifiers 用于判断 Cmd+A/C/V/X 等快捷键。
/// 返回 true = 事件已被消耗。
pub fn on_key(&mut self, kc: KeyCode, modifiers: Modifiers) -> bool

/// 鼠标按下：定位光标、清除选区、开始拖选
pub fn on_mouse_down(&mut self, px: f32, py: f32) -> bool

/// 鼠标拖拽：扩展选区
pub fn on_mouse_drag(&mut self, px: f32, py: f32)

/// 鼠标释放：结束拖选
pub fn on_mouse_up(&mut self)
```

### 键盘逻辑

| 输入 | 行为 |
|------|------|
| InsertChar(c) | 如有选区则替换选区；在 cursor 位置插入字符；cursor+1；fire on_changed |
| Backspace | 如有选区则删除选区；否则删 cursor 前一字符（注意 UTF-8 边界）；fire on_changed |
| Enter | fire on_enter |
| Escape | fire on_escape |
| Tab | 不处理，返回 false |
| Left/Right | 移动光标（有 Shift 则扩展选区） |
| Home/End | 行首/行尾 |
| Cmd+A | 全选 |
| Cmd+C | 读选区文本 → fire on_copy |
| Cmd+X | 读选区文本 → fire on_cut；删除选区 → fire on_changed |
| Cmd+V | fire on_paste → 插入返回的文本 → fire on_changed |
| Cmd+Left/Right | 按词移动光标 |

### 绘制

```rust
/// 在 rect 范围内绘制：
///   1. 输入区域背景（比 pill 略浅的 rounded rect）
///   2. 边框（聚焦时高亮色，失焦时 border 色）
///   3. 选区高亮（如有）
///   4. 文本或 placeholder（空文本 + 无 IME preedit 时）
///   5. IME preedit 文本 + 下划线标记
///   6. 光标（blink_on + focused + 无选区时）
/// 光标和文本位置使用 layout 阶段缓存的 cursor_x。
pub fn paint(&self, ctx: &mut PaintCtx)
```

### 数据同步

```rust
/// 从外部数据源同步文本。仅当 ext_text 与内部 text 不一致时覆盖。
/// 用于解决 SearchBarSnapshot 每帧注入与 TextBox 自管理状态的冲突。
pub fn sync_text(&mut self, ext_text: &str)
```

**为什么需要**：App 每帧根据 SearchState 构造 SearchBarSnapshot 下发到 SearchBarWidget。由于 TextBox 自管理状态，如果直接覆盖会导致刚输入的内容被 snap 覆盖。`sync_text` 仅在外部值与内部值不一致时才覆盖（例如 undo/redo 后 buffer 回退），保护正常输入不被打断。

## 溢出

现阶段不处理文本溢出。文本超出输入区域直接绘制，不裁剪、不滚动。

## SearchBarWidget 重构

SearchBarWidget 内部持有两个 TextBox：

```rust
struct SearchBarWidget {
    find_box: TextBox,
    replace_box: TextBox,
    // 按钮 rect、hover 状态等不变
}
```

SearchBarWidget::paint 中：
- 画背景 pill
- 画搜索图标 "/"
- 调 `find_box.paint(ctx)`
- 画 "→" 分隔符（replace 模式）
- 调 `replace_box.paint(ctx)`
- 画右侧按钮

SearchBarWidget::on_event 中：
- KeyDown：根据 focus 状态路由到 `find_box.on_key()` 或 `replace_box.on_key()`
- ImePreedit / ImeCommit / ImeEnable / ImeDisable：路由到当前 focused 的 TextBox 的 `on_ime()`
- MouseDown：hit-test 判断落在哪个 TextBox 或按钮

SearchBarWidget::set_input 中：
- 对每个 TextBox 调用 `sync_text(snap.query)` / `sync_text(snap.replace_query)`

TextBox 的 callback 在 SearchBarWidget 构造时设置，直接修改 SearchBarWidget 的 snap 状态，无需 SearchBarAction 枚举的 InsertChar/InsertReplaceChar/Backspace/ReplaceBackspace 等变体（这些变体随重构移除）。

## 文本操作注意事项

- **UTF-8 边界**：Backspace/Delete 需处理多字节字符（char 边界），光标移动需处理 grapheme cluster
- **文本变更通知**：所有修改文本的方法都触发 `on_changed(&str)`，传入新文本引用
- **焦点切换**：TextBox 失去焦点时清除选区、重置拖选状态
