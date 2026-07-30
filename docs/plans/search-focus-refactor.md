# Search Focus Refactor — 执行计划

## 目标

解耦搜索面板**可见性**与**键盘焦点**，重构键盘路由链路，规范搜索打开时的光标/交互行为。

## 当前架构问题

```
键盘事件 → key_to_command → EditCommand → handle_command Phase 4 → 译回 KeyCode → forward_key
                                                       ↑
                                            30 行"反向翻译"，根源在路由太晚
```

`ui_shell.rs` `update_frame`：
- 面板可见 → 强设 `keyboard_focus = SEARCH_BAR`
- 面板隐藏 → 强设 `keyboard_focus = None`
- "可见性绑架焦点"，导致焦点无法独立控制

## 目标行为

```
                面板关闭              面板打开
             ─────────────         ─────────────────────
             焦点在编辑器           焦点在编辑器    焦点在搜索框
             ─────────────        ─────────────   ─────────────
编辑器光标    正常闪烁              正常闪烁         静态 dimmed
搜索框光标    不存在                不存在           正常闪烁
Enter         编辑器换行            编辑器换行        Find Next
方向键        移动编辑器光标         移动编辑器光标    搜索框内移动
点击编辑器    —                    焦点回编辑器↑     焦点回编辑器
点击搜索框    —                    焦点进搜索框↓     焦点进搜索框
Cmd+F         打开面板+焦点搜框     焦点进搜索框      焦点进搜索框
Escape        —                    关闭面板          两段式关闭
```

---

## Phase 1 — 键盘路由重构（核心，优先）

### 1.1 入口层路由 (`app_lifecycle.rs:223`)

在 `KeyboardInput` 事件入口，搜索框有焦点时走快速路径：

```
KeyboardInput (pressed)
  │
  ├── keyboard_focus == SEARCH_BAR？
  │     │
  │     ├── YES → 检查键盘组合
  │     │        │
  │     │        ├── 透传白名单命中 → key_to_command（正常链路）
  │     │        │
  │     │        └── 未命中 → event.text 优先提取字符
  │     │              │
  │     │              ├── text 非空 + 非控制字符 → KeyCode::Char(c)
  │     │              │        + forward_key → SearchBarWidget
  │     │              │
  │     │              └── text 为空（控制键）→ winit NamedKey → KeyCode
  │     │                       + forward_key → SearchBarWidget
  │     │
  │     └── NO → key_to_command → handle_command（当前逻辑不变）
```

#### 新增函数 `winit_key_to_keycode(event: &KeyEvent) -> Option<KeyCode>`

**设计原则**：优先使用 winit 0.30 `event.text`（`Option<SmolStr>`），它已考虑键盘布局和 modifier（如 Shift+1 → `!`）。只有控制键（`text` 为 `None`）才做 NamedKey 匹配。

```rust
fn winit_key_to_keycode(event: &winit::event::KeyEvent) -> Option<KeyCode> {
    // Step 1: 优先用 event.text（自动处理 Shift 组合、非英文布局）
    if let Some(text) = &event.text {
        if !text.is_empty() {
            if let Some(ch) = text.chars().next() {
                // 过滤控制字符
                if !ch.is_control() || ch == '\t' {
                    return Some(KeyCode::Char(ch));
                }
            }
        }
    }

    // Step 2: text 为空时，按 NamedKey 匹配控制键
    use winit::keyboard::NamedKey;
    match &event.logical_key {
        Key::Named(NamedKey::Escape)     => Some(KeyCode::Escape),
        Key::Named(NamedKey::Enter)      => Some(KeyCode::Enter),
        Key::Named(NamedKey::Backspace)  => Some(KeyCode::Backspace),
        Key::Named(NamedKey::Delete)     => Some(KeyCode::Delete),
        Key::Named(NamedKey::Tab)        => Some(KeyCode::Tab),
        Key::Named(NamedKey::ArrowUp)    => Some(KeyCode::Up),
        Key::Named(NamedKey::ArrowDown)  => Some(KeyCode::Down),
        Key::Named(NamedKey::ArrowLeft)  => Some(KeyCode::Left),
        Key::Named(NamedKey::ArrowRight) => Some(KeyCode::Right),
        Key::Named(NamedKey::Home)       => Some(KeyCode::Home),
        Key::Named(NamedKey::End)        => Some(KeyCode::End),
        Key::Named(NamedKey::PageUp)     => Some(KeyCode::PageUp),
        Key::Named(NamedKey::PageDown)   => Some(KeyCode::PageDown),
        _ => None,
    }
}
```

#### 透传白名单（搜索框焦点时不拦截的快捷键）

判断条件：直接用 `modifiers` + `logical_key` 做精确匹配，不绕 `key_to_command`。

| 快捷键 | 动作 | 匹配条件 |
|--------|------|----------|
| `Cmd+F` | 切换搜索/焦点回搜框 | `super_key && Character("f")` |
| `Cmd+Shift+F` | 打开替换模式 | `super_key && shift && Character("f")` |
| `Cmd+S` | 保存 | `super_key && Character("s")` |
| `Cmd+Shift+S` | 另存为 | `super_key && shift && Character("s")` |
| `Cmd+W` | 关标签 | `super_key && Character("w")` |
| `Cmd+Z` | 撤销 | `super_key && Character("z")` |
| `Cmd+Shift+Z` | 重做 | `super_key && shift && Character("z")` |
| `Cmd+[` | 导航后退 | `super_key && Character("[")` |
| `Cmd+]` | 导航前进 | `super_key && Character("]")` |
| `Cmd+Shift+[` | 上一标签 | `super_key && shift && Character("[")` |
| `Cmd+Shift+]` | 下一标签 | `super_key && shift && Character("]")` |
| `Cmd+Option+←` | 上一标签 | `super_key && alt && ArrowLeft` |
| `Cmd+Option+→` | 下一标签 | `super_key && alt && ArrowRight` |

**关键区分**（避免误拦截 TextBox 内文本导航）：

| 快捷键 | 走向 | 原因 |
|--------|------|------|
| `Cmd+Option+←/→` | whitelist → key_to_command → tab switch | 同时按住 Cmd+Option |
| `Option+←/→` | forward_key → TextBox 按词跳转 | 仅 Option，不触发 whitelist |
| `Cmd+←/→` | forward_key → TextBox 行首行尾 | 仅 Cmd，`NamedKey` 不匹配 whitelist |
| `Ctrl+A/E` | forward_key → TextBox | 无 super_key，不触发 whitelist |

白名单检查在 `winit_key_to_keycode` 调用之前。`mods.super_key()` 为 true 时才检查字符/箭头组合，精确区分 `Cmd+Option`（tab switch）和 `Option` 单独（TextBox 按词跳转）。

### 1.2 删除 Phase 4 反向翻译 (`app_dispatch.rs`)

删除 `handle_command` 中整个 `is_search_focus` 块（约第 938-985 行）及其注释 `// ── Phase 4：keyboard forwarding 前置短路 ──`。

### 1.3 Escape 处理统一 (`app_dispatch.rs`)

```
Escape 到达 handle_command
  │
  ├── keyboard_focus == SEARCH_BAR？
  │     YES → 已由 Phase 1 入口路由转发给 TextBox
  │           TextBox.on_escape → SearchBarAction::DismissOrClear
  │           return
  │
  ├── 搜索面板可见？
  │     YES → 关闭面板（dismiss_or_clear: query 为空时 clear 关闭面板）
  │           return
  │
  └── 编辑器 Escape 逻辑（Sidebar 折叠 / 退出等）
```

### 1.4 确认 TextBox 支持的 KeyCode

| KeyCode | TextBox 行为 | 状态 |
|---------|-------------|------|
| `Escape` | `on_escape` 回调 | ✓ |
| `Enter` | `on_enter` 回调 | ✓ |
| `Backspace` | 删前一字 | ✓ |
| `Delete` | 删后一字 | ✓ |
| `Tab` | 需补：find ↔ replace 焦点切换 | **待补** |
| `Arrow Up/Down/Left/Right` | 移动光标 | ✓（TextBox 内置） |
| `Home/End` | 行首/行尾 | ✓ |
| `Char(c)` | 插入字符 | ✓ |

**改动文件：** `app_lifecycle.rs`、`app_dispatch.rs`

---

## Phase 2 — 焦点/光标解耦

### 2.1 面板可见性 ≠ 键盘焦点 (`ui_shell.rs`)

当前 `update_frame` 强绑定改为事件驱动：

```rust
// 搜索面板从不可见 → 可见时（首次打开），焦点自动进搜索框
if inputs.search_visible && !self.last_search_visible {
    self.keyboard_focus = Some(SEARCH_BAR);
}
// 搜索面板从可见 → 不可见时，清除焦点
if !inputs.search_visible && self.last_search_visible {
    self.keyboard_focus = None;
}
// 保存状态
self.last_search_visible = inputs.search_visible;
```

`last_search_visible: bool` 为 `UiShell` 新增字段。

**关键：不再在每帧强设焦点。`keyboard_focus` 状态由事件驱动改变（Phase 3）。**

### 2.2 编辑器光标：搜索框焦点时 dimmed (`render_pipeline.rs`)

```rust
let cursor_is_dimmed = self.keyboard_focus == Some(SEARCH_BAR);
let cursor_visible = cursor_is_dimmed || blink_phase_visible;
let cursor_color = if cursor_is_dimmed {
    theme.cursor_color_with_alpha(0.4)  // 常亮、半透明
} else {
    theme.cursor_color  // 正常颜色
};
```

`cursor_is_dimmed` 由 `app_renderer.rs` 在构建 `RenderContext` 时传入。

### 2.3 光标闪烁：搜索框焦点时跳过 (`app_lifecycle.rs`)

在 `about_to_wait` 的光标 blink 检测处：

```rust
if self.window_focused && self.ui_shell.keyboard_focus != Some(SEARCH_BAR) {
    // 现有 blink 检测逻辑
}
```

**改动文件：** `ui_shell.rs`、`render_pipeline.rs`、`app_renderer.rs`、`app_lifecycle.rs`

---

## Phase 3 — 焦点出口与交互

### 3.1 点击编辑器内容区 → 焦点回编辑器 (`mouse.rs`)

```rust
if self.ui_shell.keyboard_focus == Some(SEARCH_BAR)
    && hit_is_on_editor_fill_area {
    self.ui_shell.keyboard_focus = None;
    // 不关闭搜索面板，搜索高亮保留
}
```

### 3.2 点击搜索框 → 焦点进搜索框

`ui_shell.dispatch()` 中 SearchBar widget 消费事件时：

```rust
if matches!(&action, WidgetAction::SearchBar(_)) {
    self.keyboard_focus = Some(SEARCH_BAR);
}
```

**焦点子状态（find vs replace）**：
- `SearchBarWidget` 内部通过 `SearchBarSnapshot.focus_replace: bool` 维护
- 点击 Replace 输入框 → widget 内部设 `focus_replace = true`，产出 `SearchBarAction::FocusReplace`
- `keyboard_focus` 始终保持 `SEARCH_BAR`，子状态由 widget 内部闭环
- Tab 键切换：`KeyCode::Tab → forward_key → on_event`，Textbox 内切换 `focus_replace`，产出 `FocusFind`/`FocusReplace`

### 3.3 Cmd+F 已有焦点时切回搜索框

`execute_commands` 或 `handle_command` 的 `ToggleFind` 处理：

```rust
AppCommand::ToggleFind => {
    if let Some(dv) = self.active_doc_mut() {
        if dv.search_state.panel_visible {
            // 面板已开 → 焦点切回搜索框
            self.ui_shell.keyboard_focus = Some(SEARCH_BAR);
            self.needs_redraw = true;
            return;
        }
        // 面板关闭 → 打开面板（Phase 2.1 自动设焦点）
        dv.search_state.panel_visible = !dv.search_state.panel_visible;
        // ...
    }
}
```

**改动文件：** `mouse.rs`、`ui_shell.rs`、`app_dispatch.rs`

---

## 不改的文件

| 文件 | 原因 |
|------|------|
| `input.rs` / `key_to_command` | 不上游做焦点判断，保持无状态 |
| `search_bar.rs` | SearchBarWidget 本身不变，上层路由方式变 |
| `search_state.rs` | 内部状态不变 |
| `app_search.rs` | 搜索/替换逻辑不变 |
| `commands.rs` | 编辑命令不变 |

---

## 风险与边界

- **Ctrl+A/E 等 Emacs 风格快捷键**：搜索框焦点时通过 forward_key 进入 TextBox。TextBox 如支持这些快捷键则自然可用；不支持则暂被忽略（不插入字符因为它走 `Char` 路径？实际上 Ctrl+A 在 winit 产生 `event.text = "\u{01}"` 控制字符 → `winit_key_to_keycode` 中 `ch.is_control()` 过滤 → 返回 `None` → 按键被丢弃）。如果需要支持，后续在 TextBox 的 `on_event` 中扩展。
- **Enter + Shift**：当前 `find_box.on_enter` 固定走 `SearchBarAction::Next`。后续可加 `Shift+Enter → Prev`（本期不强制）。
