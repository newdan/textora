# 阶段 11：应用菜单系统开发方案

> 参考：Zed 的 `ApplicationMenu` + `ContextMenu` + `PopoverMenu` 实现
> 分析来源：`plans_zed_menu_tabbar_analysis.md`
> 约束：每阶段改动 ≤ 3 个文件；每阶段独立可编译/可测试；不改 3+ 文件原则

---

## 一、整体架构设计

### 1.1 Zed 菜单系统的分层

```
┌─────────────────────────────────────────────────┐
│ app_menus.rs                                     │
│ 定义菜单结构：Vec<Menu>                          │
│ Menu { name, items: Vec<MenuItem> }              │
│ MenuItem = Action | Separator | Submenu          │
├─────────────────────────────────────────────────┤
│ application_menu.rs                              │
│ 渲染菜单栏 + 管理展开/切换状态                    │
│ 使用 PopoverMenu + ContextMenu 渲染下拉          │
├─────────────────────────────────────────────────┤
│ context_menu.rs (GPUI 组件)                      │
│ 下拉菜单的具体渲染：列表 + 键盘导航 + 子菜单     │
├─────────────────────────────────────────────────┤
│ menu crate                                       │
│ 菜单动作定义：Cancel/Confirm/SelectNext/etc.     │
└─────────────────────────────────────────────────┘
```

### 1.2 edit+ 适配设计

edit+ 用的是 **wgpu + cosmic-text** 直接渲染，没有 GPUI 的 Entity/PopoverMenu/ContextMenu 组件体系。需要用 GPU quads + 文字布局 + 事件处理来等价实现。

**核心模块拆分**：

```
crates/app/src/
├── menu_model.rs    # 菜单数据模型（Menu, MenuItem, MenuAction）
├── menu_bar.rs      # 菜单栏渲染 + 交互（水平菜单条）
├── menu_popup.rs    # 下拉菜单渲染 + 交互（弹出列表）
└── app.rs           # 集成：事件分发、布局协调
```

### 1.3 数据流

```
用户点击/键盘 → App::handle_input()
                    │
          ┌─────────┴─────────┐
          │                   │
    菜单栏区域?           编辑器区域?
          │                   │
    menu_bar::             正常编辑
    hit_test()              事件处理
          │
    ┌─────┴─────┐
    │           │
 菜单标题    下拉项目
    │           │
 展开/切换   执行 action
 下拉菜单    + 关闭菜单
```

---

## 二、阶段拆分

### 11.1 菜单数据模型（`menu_model.rs`）

**目的**：定义菜单的数据结构，与 Zed 的 `Menu`/`MenuItem` 对齐但简化。

#### 设计

```rust
// crates/app/src/menu_model.rs

use crate::commands::EditCommand;

/// 一个完整的菜单（如 "File"、"Edit"）
#[derive(Debug, Clone)]
pub struct Menu {
    pub name: String,
    pub items: Vec<MenuItem>,
}

/// 菜单项
#[derive(Debug, Clone)]
pub enum MenuItem {
    /// 分隔线
    Separator,
    /// 可点击的动作项
    Action {
        label: String,
        /// 快捷键显示文本（如 "⌘S"），仅用于显示，不参与事件分发
        shortcut: Option<String>,
        /// 关联的编辑命令或应用动作
        command: MenuCommand,
        /// 是否禁用（灰色显示，不可点击）
        disabled: bool,
    },
    /// 子菜单
    Submenu(Menu),
}

/// 菜单动作 —— 既可以是已有的 EditCommand，也可以是新的应用级动作
#[derive(Debug, Clone)]
pub enum MenuCommand {
    /// 复用已有的编辑命令（如 Save, Undo, Cut, Copy, Paste 等）
    Edit(EditCommand),
    /// 应用级动作（如 NewFile, OpenFile, CloseTab, Quit 等）
    App(AppAction),
}

/// 应用级动作
#[derive(Debug, Clone)]
pub enum AppAction {
    NewFile,
    OpenFile,
    Save,
    SaveAs,
    CloseTab,
    CloseWindow,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Find,
    FindNext,
    FindPrevious,
    Replace,
    ToggleTabBar,
    ToggleStatusBar,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

impl MenuCommand {
    /// 人类可读的快捷键显示文本
    pub fn default_shortcut(&self) -> Option<&'static str> {
        match self {
            MenuCommand::Edit(EditCommand::Save) => Some("⌘S"),
            MenuCommand::App(AppAction::Undo) => Some("⌘Z"),
            MenuCommand::App(AppAction::Redo) => Some("⌘⇧Z"),
            MenuCommand::App(AppAction::Cut) => Some("⌘X"),
            MenuCommand::App(AppAction::Copy) => Some("⌘C"),
            MenuCommand::App(AppAction::Paste) => Some("⌘V"),
            MenuCommand::App(AppAction::SelectAll) => Some("⌘A"),
            MenuCommand::App(AppAction::Find) => Some("⌘F"),
            MenuCommand::App(AppAction::NewFile) => Some("⌘N"),
            MenuCommand::App(AppAction::OpenFile) => Some("⌘O"),
            MenuCommand::App(AppAction::CloseTab) => Some("⌘W"),
            MenuCommand::App(AppAction::Quit) => Some("⌘Q"),
            MenuCommand::App(AppAction::SaveAs) => Some("⌘⇧S"),
            _ => None,
        }
    }
}
```

#### 默认菜单结构

```rust
/// 构建 edit+ 的默认菜单栏
pub fn build_default_menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New",       AppAction::NewFile),
                MenuItem::action("Open...",   AppAction::OpenFile),
                MenuItem::separator(),
                MenuItem::action("Save",      AppAction::Save),
                MenuItem::action("Save As...", AppAction::SaveAs),
                MenuItem::separator(),
                MenuItem::action("Close Tab", AppAction::CloseTab),
                MenuItem::action("Close Window", AppAction::CloseWindow),
                MenuItem::separator(),
                MenuItem::action("Quit",      AppAction::Quit),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::action("Undo",      AppAction::Undo),
                MenuItem::action("Redo",      AppAction::Redo),
                MenuItem::separator(),
                MenuItem::action("Cut",       AppAction::Cut),
                MenuItem::action("Copy",      AppAction::Copy),
                MenuItem::action("Paste",     AppAction::Paste),
                MenuItem::separator(),
                MenuItem::action("Select All", AppAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find...",   AppAction::Find),
                MenuItem::action("Find Next", AppAction::FindNext),
                MenuItem::action("Find Previous", AppAction::FindPrevious),
                MenuItem::action("Replace...", AppAction::Replace),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Zoom In",   AppAction::ZoomIn),
                MenuItem::action("Zoom Out",  AppAction::ZoomOut),
                MenuItem::action("Reset Zoom", AppAction::ZoomReset),
                MenuItem::separator(),
                MenuItem::action("Toggle Tab Bar", AppAction::ToggleTabBar),
                MenuItem::action("Toggle Status Bar", AppAction::ToggleStatusBar),
            ],
        },
    ]
}
```

#### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/menu_model.rs` | **新增** — Menu/MenuItem/MenuCommand/AppAction 定义 + 默认菜单构建函数 |

#### 验收

- 编译通过
- 默认菜单结构可被后续模块引用

---

### 11.2 菜单栏水平渲染（`menu_bar.rs`）

**目的**：在窗口顶部渲染水平菜单条，每个菜单标题可点击/悬停。

#### 设计

参考 Zed 的 `ApplicationMenu::render()` — 水平排列菜单标题，悬停时高亮，点击时展开下拉。

```rust
// crates/app/src/menu_bar.rs

use crate::menu_model::Menu;

/// 菜单栏布局结果
#[derive(Debug, Clone)]
pub struct MenuBarLayout {
    /// 每个菜单标题的布局信息
    pub menus: Vec<MenuTitleEntry>,
    /// 菜单栏高度（像素）
    pub height: f32,
}

#[derive(Debug, Clone)]
pub struct MenuTitleEntry {
    pub index: usize,
    pub label: String,
    /// 标题在 NDC 中的矩形区域 [left, right, top, bottom]
    pub rect: [f32; 4],
}

/// 菜单栏状态
#[derive(Debug, Clone)]
pub struct MenuBarState {
    /// 当前悬停的菜单索引
    pub hovered_index: Option<usize>,
    /// 当前展开的菜单索引
    pub open_index: Option<usize>,
}

impl MenuBarState {
    pub fn new() -> Self {
        Self { hovered_index: None, open_index: None }
    }
}
```

#### 渲染层级

```
┌─────────────────────────────────────────────────┐
│  [menu_bar_bg: 24px 高, 深色条]                  │
│  File    Edit    View    Help                    │  ← 菜单标题（cosmic-text 渲染）
│  ─────────────────────────────────────────────── │  ← 1px 分割线
│  ┌──────────┬──────────┬──────────┐             │  ← TabBar 区域
│  │ main.rs  │ Cargo    │ README   │ [×]         │
│  └──────────┴──────────┴──────────┘             │
└─────────────────────────────────────────────────┘
```

#### 交互逻辑

```
悬停 → 标题高亮（背景色变化）
      └─ 如果已有其他菜单展开 → 切换展开 + 关闭旧下拉

点击 → 切换展开/关闭当前菜单下拉
      └─ 已展开时再点击 → 关闭

点击菜单栏外部（编辑区/TabBar） → 关闭所有下拉
```

#### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/menu_bar.rs` | **新增** — `layout_menus()`（计算标题位置）、`menu_bar_vertices()`（GPU quads）、`hit_test_menu_bar()`（点击检测） |
| `crates/app/src/app.rs` | 渲染管线增加 `menu_bar_vertices()` 调用；窗口 resize 时触发布局重算；在 mouse/click 事件中先过 `hit_test_menu_bar()` |

#### 验收

- 菜单栏水平显示在窗口顶部
- 悬停菜单标题 → 背景高亮
- 点击菜单标题 → 状态标记为 "展开"（下拉在下个阶段实现）
- 点击菜单栏外部 → 状态重置

---

### 11.3 下拉菜单弹出（`menu_popup.rs`）

**目的**：点击/悬停菜单标题时，在标题下方弹出下拉列表，显示菜单项。

#### 设计

这是最复杂的部分。Zed 的 `ContextMenu` 是一个完整的 GPUI Entity，edit+ 需要用 GPU quads + hit-test 等价实现。

```rust
// crates/app/src/menu_popup.rs

/// 下拉菜单布局
#[derive(Debug, Clone)]
pub struct PopupLayout {
    /// 背景矩形 [left, right, top, bottom] (NDC)
    pub bg_rect: [f32; 4],
    /// 每个菜单项的布局
    pub items: Vec<PopupItemEntry>,
    /// 子菜单（如果有的话）
    pub submenu: Option<Box<PopupLayout>>,
}

#[derive(Debug, Clone)]
pub struct PopupItemEntry {
    pub index: usize,
    pub label: String,
    pub shortcut: Option<String>,
    pub is_separator: bool,
    pub is_disabled: bool,
    pub has_submenu: bool,
    /// 项目的矩形区域 (NDC)
    pub rect: [f32; 4],
}

/// 下拉菜单交互状态
#[derive(Debug, Clone)]
pub struct PopupState {
    /// 当前高亮项索引（键盘/鼠标导航）
    pub selected_index: Option<usize>,
    /// 子菜单状态
    pub submenu: Option<SubmenuState>,
}

#[derive(Debug, Clone)]
pub struct SubmenuState {
    pub parent_item_index: usize,  // 触发子菜单的父菜单项索引
    pub popup: PopupState,         // 子菜单的交互状态
}
```

#### 渲染要点

```
┌──────────────────────────┐
│ New          ⌘N         │  ← 选中项（蓝色高亮）
│ Open...      ⌘O         │
├──────────────────────────┤  ← 分隔线
│ Save         ⌘S         │
│ Save As...   ⌘⇧S       │
├──────────────────────────┤
│ Close Tab    ⌘W    ▸    │  ← 有子菜单（▸ 箭头）
│ Close Window            │
├──────────────────────────┤
│ Quit         ⌘Q         │
└──────────────────────────┘
```

**子菜单展开**（参考 Zed 的 `SubmenuState::Open`）：

```
┌──────────────┐
│ View         │
│ Zoom In  ▸ ──┼──────────────┐
│ Zoom Out     │ 150%         │  ← 子菜单在右侧弹出
│ Reset Zoom   │ 200%         │
└──────────────┘ 300%         │
                  └────────────┘
```

子菜单定位规则：
- 默认从父菜单项右侧弹出
- 如果右侧超出窗口 → 从左侧弹出（Zed 的 `flip_left` 逻辑）

#### 键盘导航（参考 Zed `menu` crate 的动作）

| 按键 | 动作 | 行为 |
|---|---|---|
| `↓` | SelectNext | 选中下一项（跳过禁用项和分隔线） |
| `↑` | SelectPrevious | 选中上一项 |
| `Enter` | Confirm | 执行选中项的动作 |
| `→` | SelectChild | 展开子菜单（如果有） |
| `←` | SelectParent | 关闭子菜单 / 回到父菜单 |
| `Escape` | Cancel | 关闭下拉 |
| `Home` | SelectFirst | 选中第一项 |
| `End` | SelectLast | 选中最后一项 |

#### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/menu_popup.rs` | **新增** — `layout_popup()`、`popup_vertices()`、`hit_test_popup()`、`PopupState` |
| `crates/app/src/menu_bar.rs` | 新增 `open_popup_index: Option<usize>` + 下拉渲染调用 |
| `crates/app/src/app.rs` | `WindowEvent::KeyboardInput` 在菜单打开时先分发给菜单（拦截 ↑↓Enter Escape），不再传递给编辑器；`MouseInput` 先检查是否命中弹出区域 |

#### 注意

> ⚠️ 此阶段改动涉及 3 个文件（menu_popup.rs 新增 + menu_bar.rs + app.rs），刚好触及上限。
> 如果子菜单功能导致复杂度超标，可以将子菜单推迟到 11.4。

#### 验收

- 点击 "File" → 下拉菜单出现在标题下方
- 菜单项显示 label + shortcut
- 分隔线正确渲染
- 点击菜单项 → 执行对应 command → 关闭菜单
- 点击菜单外部 → 关闭菜单
- ↑↓ 键导航 → 选中项高亮
- Enter → 执行选中项
- Escape → 关闭菜单

---

### 11.4 子菜单（嵌套菜单）

**目的**：支持菜单项 → 展开子菜单（如 "View → Zoom → 150%"）。

#### 设计

构建在 11.3 之上，PopupState 已有 `submenu: Option<SubmenuState>` 字段。本阶段补齐：

1. 子菜单布局：从父菜单项右侧偏移弹出
2. 翻转逻辑：右侧空间不足时翻到左侧
3. 交互：`→` 展开、`←` 关闭、鼠标悬停切换
4. 级联关闭：关闭父菜单时递归关闭子菜单

#### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/menu_popup.rs` | 新增 `layout_submenu()`、`flip_direction()`；修改状态机支持嵌套 |
| `crates/app/src/menu_model.rs` | `MenuItem::Submenu(Menu)` 变体（11.1 已预留） |

#### 验收

- 子菜单从父菜单项右侧/左侧正确弹出
- 鼠标悬停父菜单项 → 展开子菜单
- 鼠标移开 → 子菜单保持打开（仅在移入另一个子菜单项时才切换）
- `→` 键展开子菜单
- `←` 键关闭子菜单回到父菜单
- 递归深度 ≥ 2（子菜单里再有子菜单）

---

### 11.5 菜单栏悬停切换 + 快捷键集成

**目的**：完善菜单栏交互——鼠标在菜单标题间移动时自动切换下拉 + 将菜单 command 与现有快捷键系统统一分发。

#### 设计

**悬停切换**（参考 Zed `ApplicationMenu::render_standard_menu` 的 `on_hover`）：

```
用户在 "File" 展开状态 → 鼠标移到 "Edit" 标题
  → on_hover("Edit") 触发
  → 关闭 "File" 下拉
  → 打开 "Edit" 下拉
```

**实现**：在 `App::handle_cursor_moved()` 中，如果菜单栏处于打开状态，每次鼠标移动检测 hover 是否进入另一个菜单标题的矩形区域。

**快捷键集成**：

```rust
impl App {
    fn dispatch_menu_command(&mut self, cmd: &MenuCommand) {
        match cmd {
            MenuCommand::Edit(ec) => self.execute_edit_command(ec),
            MenuCommand::App(AppAction::NewFile) => self.new_empty_tab(),
            MenuCommand::App(AppAction::OpenFile) => self.open_file_dialog(),
            MenuCommand::App(AppAction::Quit) => self.running = false,
            // ...
        }
    }
}
```

#### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/menu_bar.rs` | `MenuBarState` 增加悬停切换逻辑 |
| `crates/app/src/app.rs` | 新增 `dispatch_menu_command()`；`CursorMoved` 事件增加菜单栏悬停检测 |

#### 验收

- 展开 "File" → 鼠标移到 "Edit" 标题 → "File" 下拉关闭，"Edit" 下拉打开
- 菜单 command 执行的行为与对应快捷键一致（如 "Save" 菜单项 = ⌘S）

---

### 11.6 视觉打磨

**目的**：菜单渲染的细节优化，使其看起来接近原生应用。

#### 设计

| 细节 | 实现 |
|---|---|
| 阴影/边框 | 下拉菜单外围 1px 边框 + 2px 偏移阴影（用半透明 quads 模拟） |
| 选中项高亮 | 蓝色圆角矩形背景（`theme.selection` 色，alpha 0.3） |
| disabled 项 | 灰色文字（`theme.foreground` alpha 0.4） |
| 分隔线 | 1px 水平线（`theme.border` 色），上下各 4px padding |
| 子菜单箭头 | cosmic-text 渲染 `▸` 字符，右对齐 |
| 快捷键文本 | 右对齐，灰色，比 label 颜色浅 |
| 动画（可选） | 下拉菜单简单的 fade-in（alpha 0→1，3 帧） |

#### 波及文件

| 文件 | 改动 |
|---|---|
| `crates/app/src/menu_popup.rs` | 阴影/圆角/高亮/disalbed 渲染增强 |

#### 验收

- 下拉菜单有 1px 边框 + 微弱阴影
- 选中项有蓝色圆角矩形背景
- disabled 项灰色不可点击
- 快捷键右对齐显示
- 子菜单项有 `▸` 箭头

---

## 三、数据流总览

```
                    ┌──────────────────┐
                    │   menu_model.rs  │
                    │   Menu / MenuItem│
                    │   MenuCommand    │
                    └────────┬─────────┘
                             │ 提供数据
          ┌──────────────────┼──────────────────┐
          │                                     │
   menu_bar.rs                          menu_popup.rs
   ┌────────────────┐                  ┌────────────────┐
   │ layout_menus() │                  │ layout_popup() │
   │ menu_bar_vert()│                  │ popup_vertices │
   │ hit_test()     │                  │ hit_test()     │
   │ MenuBarState   │                  │ PopupState     │
   └───────┬────────┘                  └───────┬────────┘
           │                                   │
           └───────────────┬───────────────────┘
                           │ 事件分发
                    ┌──────┴──────┐
                    │   app.rs    │
                    │   handle_   │
                    │   input()   │
                    │   dispatch_ │
                    │   menu_     │
                    │   command() │
                    └─────────────┘
```

## 四、跨阶段依赖

```
11.1 数据模型 ──→ 11.2 菜单栏 ──→ 11.3 下拉弹出 ──→ 11.4 子菜单
                                        │                  │
                                        └──→ 11.5 悬停   ←┘
                                             切换+快捷键
                                                  │
                                                  └──→ 11.6 视觉打磨
```

11.4 和 11.5 可以并行做。

## 五、实施优先级

| 优先级 | 阶段 | 收益 | 改动量 | 依赖 |
|---|---|---|---|---|
| **P0** | 11.1 数据模型 | 所有阶段的基石 | ~150 行（新建） | 无 |
| **P0** | 11.2 菜单栏渲染 | 水平菜单条可见 | ~200 行（新建+改动） | 11.1 |
| **P1** | 11.3 下拉弹出 | 核心交互完成 | ~350 行（新建+改动） | 11.2 |
| **P2** | 11.4 子菜单 | 嵌套菜单 | ~150 行 | 11.3 |
| **P2** | 11.5 悬停切换+快捷键 | 交互流畅性 | ~100 行 | 11.3 |
| **P3** | 11.6 视觉打磨 | 原生感 | ~100 行 | 11.3 |

## 六、关键边界情况

| # | 场景 | 预期行为 |
|---|---|---|
| 1 | 菜单项数 > 屏幕高度 | 不截断，如果实在太高则做滚动（可推迟到 P3） |
| 2 | 子菜单递归深度 > 3 | 允许，但建议限制深度为 3（Zed 也有限制） |
| 3 | 打开菜单时 resize 窗口 | 关闭所有菜单，重新布局 |
| 4 | 菜单项 label 超长 | 最小菜单宽度 200px，label 可到 400px 后截断 + "…" |
| 5 | 禁用菜单项被键盘选中 | `select_next` / `select_previous` 跳过禁用项 |
| 6 | 分隔线被键盘选中 | 跳过 |
| 7 | 菜单打开时按 Tab | 关闭菜单 |
| 8 | macOS 全屏模式 | 菜单栏在系统菜单栏下方（或隐藏系统菜单栏后显示自定义菜单栏） |

## 七、与现有快捷键系统的关系

菜单不替代快捷键。关系是**双向同步**：

- **菜单 → 编辑器**：点击菜单项 → 执行 `MenuCommand` → 如果对应 `EditCommand`，走现有的 `execute_edit_command_v2()` 路径
- **编辑器 → 菜单**：现有快捷键（如 `⌘S`）继续工作，菜单只是额外提供可视化的发现途径

不需要修改 `commands.rs` 或 `input.rs` 的现有快捷键逻辑。
