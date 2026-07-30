# Navigator Trait v2 设计

> **驱动**: 当前 Navigator trait 混杂了"导航逻辑"和"UI 表现（渲染/滚动/动画/命中测试）"，职责不清。
> TabBarNavigator 包装层在 Navigator trait 和 TabBarWidget 之间做了冗余的状态复制。

## 1. 核心原则

```
Navigator trait   — 纯数据导航（条目集合 + 激活项切换）
TabBarWidget      — 自己管渲染、布局、autoscroll 目标计算
App::SmoothScroll — 提供动画插值机制，TabBar/Sidebar 共用
```

## 2. Navigator trait（纯导航）

```rust
/// 条目投影 —— 不引用 DocumentView，UI 层可直接消费。
#[derive(Debug, Clone)]
pub struct NavEntry {
    pub title: String,
    pub file_path: Option<PathBuf>,
    pub is_dirty: bool,
    pub pinned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavEffect {
    None,
    ActiveChanged,   // 激活项变了 → REDRAW
    ItemsChanged,    // 条目增删 → REDRAW + PERSIST
}

pub trait Navigator: Any {
    fn id(&self) -> &str;
    fn name(&self) -> &str;

    /// 返回条目的当前投影。
    ///
    /// 当前每帧调用（Immediate Mode 架构下 App 构建 TabBarInput 时调用）。
    /// Vec 分配开销在几十个条目内可忽略。若未来 profiler 显示此处分配显著，
    /// 可改为迭代器形式：`fn items(&self) -> impl Iterator<Item = NavEntry> + '_`
    /// 调用方直接 map 到 TabInfo，避免中间 Vec。
    fn items(&self) -> Vec<NavEntry>;
    fn len(&self) -> usize { self.items().len() }
    fn active_index(&self) -> usize;

    fn switch_to(&mut self, index: usize) -> NavEffect;
    fn close(&mut self, index: usize) -> NavEffect;

    fn toggle_pin(&mut self, index: usize) -> NavEffect;
    fn is_pinned(&self, index: usize) -> bool;
    fn pinned_indices(&self) -> &HashSet<usize>;

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
```

**删除的方法**（与导航无关，属于 UI）：
- `render()` / `hit_test()` / `hover()`
- `scroll()` / `scroll_offset()` / `tick()` / `is_animating()`
- `thickness()`

**删除的类型**：
- `NavContext` / `NavOutput` — UI 数据流不再走 Navigator
- `NavAction` — TabBarWidget 已有 `TabBarAction`

## 3. 命名修正

| 旧名 | 新名 | 理由 |
|------|------|------|
| `Tab` | `DocItem` | 不是 UI tab，是文档条目 |
| `Workspace.tabs` | `Workspace.entries` | 同上 |
| `Workspace.tab()` | `Workspace.entry()` | 同上 |
| `new_empty_tab_with_viewport()` | `new_untitled()` | 语义：创建无标题文档 |

## 4. Workspace 实现 Navigator

```rust
pub(crate) struct Workspace {
    entries: Vec<DocItem>,
    active_index: usize,
    entry_history: Vec<usize>,
    pinned_indices: HashSet<usize>,
    back_history: Vec<usize>,
    forward_history: Vec<usize>,
    // preview_index 移出到 App（由插件行为决定，不是纯导航属性）
}

/// preview_index 迁移到 App 层：
/// App.preview_index: Option<usize>
/// 当 Markdown 插件在预览模式下打开链接时设置，
/// 切换标签时自动关闭（逻辑从 Workspace::switch_to 移到 App::handle_nav_effect）
```

**实现 Navigator trait**：
```rust
impl Navigator for Workspace {
    fn id(&self) -> &str { "builtin.files" }
    fn name(&self) -> &str { "Open Files" }

    fn items(&self) -> Vec<NavEntry> {
        self.entries.iter().enumerate().map(|(i, e)| NavEntry {
            title: e.doc_title(),
            file_path: e.doc.file_path.clone(),
            is_dirty: e.doc.dirty,
            pinned: self.pinned_indices.contains(&i),
        }).collect()
    }

    fn switch_to(&mut self, idx: usize) -> NavEffect {
        // 移植现有 switch_to() 逻辑
        // 返回值从 WorkspaceEffect → NavEffect
    }

    fn close(&mut self, idx: usize) -> NavEffect {
        // 移植现有 try_close_tab + close_tab_inner 逻辑
    }
}
```

**留在 Workspace 但不在 Navigator trait 里的**：
- `active_doc()` / `active_doc_mut()` — 文档内容访问
- `entry()` / `entry_mut()` / `entries()` — 条目访问
- `open_file()` / `new_untitled()` — 文档生命周期
- `go_back()` / `go_forward()` / `record_nav_step()` — 导航历史
- `switch_plugin()` — 插件切换（委托给 active DocItem）
- `snapshot()` / `restore_with_viewport()` — 持久化

## 5. TabBarWidget 接管滚动目标

TabBarWidget 是唯一知道"每个 tab 在哪、clip 区域多大"的人，所以 autoscroll 目标由它计算：

```rust
impl TabBarWidget {
    /// set_input 内部处理：
    /// 1. 布局所有 tab
    /// 2. 如果 active tab 不在 clip 区域内 → 更新内部 scroll_target
    /// 3. 接收外部传入的 current_scroll 用于渲染
    pub fn set_input(&mut self, input: TabBarWidgetInput, ...);

    /// 用户滚动输入（鼠标滚轮 / 快捷键）→ 更新内部 target
    pub fn scroll_by(&mut self, delta: f32);

    /// 当前滚动目标（App 读去做动画插值）
    pub fn scroll_target(&self) -> f32;
}
```

TabBarState 新增：
```rust
pub struct TabBarState {
    layout: Option<TabBarLayout>,
    scroll_offset: f32,
    scroll_target: f32,       // 新增
    hovered_index: Option<usize>,
    preview_index: Option<usize>,
    open_menu: Option<PopupMenu>,
}
```

## 6. App 层动画机制

App 提供通用的 `SmoothScroll` 结构，不绑定特定 widget：

```rust
/// 平滑滚动插值器，App 层工具。
struct SmoothScroll {
    offset: f32,
    target: f32,
}

impl SmoothScroll {
    fn new() -> Self { Self { offset: 0.0, target: 0.0 } }
    fn current(&self) -> f32 { self.offset }
    fn target(&self) -> f32 { self.target }
    fn set_target(&mut self, t: f32) { self.target = t; }

    /// 每帧调用。返回 true 表示还在动画中。
    fn tick(&mut self) -> bool {
        let diff = self.target - self.offset;
        if diff.abs() < 0.5 {
            self.offset = self.target;
            return false;
        }
        self.offset += diff * 0.35;
        true
    }

    fn is_animating(&self) -> bool {
        (self.target - self.offset).abs() >= 0.5
    }
}
```

App 渲染循环：
```rust
// app_renderer.rs — 每帧

// 1. 渲染 TabBar：传入当前动画插值 offset
let tab_scroll_offset = self.tab_scroll.current();
self.ui_shell.set_tab_bar_input(
    tabs, active_index,
    scroll_offset_px: tab_scroll_offset,
    ...
);

// 2. 渲染完成后，读 TabBar 的 target 并 tick
self.tab_scroll.set_target(self.ui_shell.tab_bar_scroll_target());

if self.tab_scroll.tick() {
    self.needs_redraw = true;
}
```

滚动输入路径（鼠标滚轮 / 快捷键）：
```rust
// 用户滚轮 → TabBarWidget 内部更新 target → App 下一帧 tick 到新 target
self.ui_shell.tab_bar_scroll_by(dx);
```

## 7. 删除清单

| 文件 | 内容 |
|------|------|
| `navigators/tab_bar.rs` | TabBarNavigator 整个删除 |
| `navigators/mod.rs` | 删除 |
| `navigator.rs` | 重写为纯 Navigator trait（仅 NavEntry + NavEffect + trait 定义） |

| 字段/方法 | 位置 | 替代 |
|-----------|------|------|
| `Workspace.navigator` | workspace.rs | 删除 |
| `Workspace.tab_scroll_offset` | （已在上一轮删除） | — |
| `WorkspaceEffect` | workspace.rs | `NavEffect` |
| `Tab::is_markdown()` | （已在上一轮删除） | — |
| `Tab::legacy_md_preview()` | tab.rs | 保留至插件架构完善后删除 |

| 调用点替换 | 旧 | 新 |
|------------|----|----|
| `ws.navigator.scroll_offset()` | app_renderer.rs:320 | `self.tab_scroll.current()` |
| `ws.navigator.tick()` | app_renderer.rs:835 | `self.tab_scroll.tick()` |
| `ws.navigator.is_animating()` | app_window.rs:232 | `self.tab_scroll.is_animating()` |
| `ws.navigator.scroll(dx)` | app_scroll.rs:131 | `self.ui_shell.tab_bar_scroll_by(dx)` |
| `ws.navigator.is_animating()` | dispatch/tabs.rs:45 | `self.tab_scroll.is_animating()` |
| `ws.navigator.scroll(delta)` | dispatch/chrome.rs:80 | `self.ui_shell.tab_bar_scroll_by(delta)` |

## 8. 多 Workspace 预留

当前不实现切换 UI，架构上预留：

```rust
pub(crate) struct App {
    workspaces: Vec<Box<dyn Navigator>>,  // 未来多个
    active_workspace: usize,              // 当前
    // ...
}

impl App {
    fn active_navigator(&self) -> &dyn Navigator { &*self.workspaces[self.active_workspace] }
    fn active_navigator_mut(&mut self) -> &mut dyn Navigator { &mut *self.workspaces[self.active_workspace] }

    // Workspace 特有的方法通过 downcast 访问
    fn workspace(&self) -> &Workspace {
        self.active_navigator().as_any().downcast_ref::<Workspace>().unwrap()
    }
}
```

TabBar/Sidebar 始终从 `app.active_navigator().items()` 读取条目，不关心底层是 FileWorkspace 还是 SearchResultsWorkspace。

## 9. 实现步骤

1. **重命名** — `Tab → DocItem`，`tabs → entries`，`tab() → entry()` 等
2. **重构 Navigator trait** — 砍掉 UI 方法，只保留纯导航接口；`NavContext/NavOutput/NavAction` 删
3. **Workspace 实现 Navigator** — 添加 `impl Navigator for Workspace`，`preview_index` 移到 App
4. **TabBarWidget 加 scroll_target** — `scroll_by()` + `scroll_target()` + autoscroll
5. **App 加 SmoothScroll** — 替换 6 处 navigator 调用点
6. **删除 TabBarNavigator** — 删文件，删 Workspace.navigator 字段
7. **WorkspaceEffect → NavEffect** — 全局替换
8. **cargo check + cargo test** — 验证

## 10. 不变项

- `crates/ui` 不依赖 `crates/app`（红线）
- `ui::tab_bar::TabBarWidget` 接口保持稳定
- `ContentPlugin` trait 不受影响
- 持久化格式不受影响
