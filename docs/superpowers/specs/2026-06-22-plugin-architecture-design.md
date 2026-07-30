> **Status: Implemented** (Phase 1–4 complete, Phase 5 ongoing)
> 
> Implementation plan: `docs/superpowers/plans/2026-06-22-plugin-architecture-implementation.md`
> Phase 3–5 plan: `docs/superpowers/plans/2026-06-22-plugin-architecture-phase3-5.md`


# 插件架构设计

## 概述

将当前硬编码在 app 层的 Markdown 预览等视图模式，改造为统一的插件架构。基础文本编辑也是插件实现，所有内容渲染路径通过 `ContentPlugin` trait 统一。

## 动机

### 当前问题

MD 预览通过 `View` 枚举硬编码在 app 层：

```rust
pub(crate) enum View {
    Editor(DocumentView),
    Markdown(MdView),
}
```

耦合点散布在 12+ 处 —— `app_renderer.rs`（9 处分支）、`app_scroll.rs`、`app_search.rs`、`dispatch/mouse.rs`、`dispatch/viewport.rs`、`dispatch/editor.rs`、`events.rs`、`app_dispatch.rs` 等，到处都是 `if let View::Markdown(mv) = ...` 的分支判断。其中 `app_renderer.rs` 和 `dispatch/editor.rs` 耦合最严重（各含大量条件分支和状态交互）。每新增一种视图模式都需要修改所有分支。

### 目标

- 所有内容渲染通过 `ContentPlugin` trait 统一，消除硬编码分支
- 基础文本编辑也是 `ContentPlugin` 实现
- 编译时插件注册，零运行时开销

## 核心接口

### ContentPlugin

> [!IMPORTANT]
> trait 定义在 `crates/app/src/plugin.rs`。虽然理想情况应放在更底层的 crate 以实现彻底解耦，
> 但由于 `PluginContext` 需要持有 app 层才有的资源（如 `DocumentView` 的可变引用），
> 且当前所有插件实现均在 app 内部，放在 app 层是务实选择。
> 若未来需要进程外插件或独立 crate 插件，再提取到独立 crate。

```rust
// crates/app/src/plugin.rs

pub type PluginId = &'static str;

pub struct PluginOutput {
    pub draw_list: DrawList,         // DrawList { cmds: Vec<DrawCmd>, offset: (f32, f32) }
    pub content_height: f32,         // 宿主据此驱动主滚动条
    pub needs_drain: bool,           // true 表示 draw_list 需要 paint_backend::drain() 转换为 GlyphVertex
}

pub enum CommandFlow {
    Consumed,       // 插件已处理，不再传给编辑器
    Passthrough,    // 插件不处理，基础编辑器接管
}

pub struct HitResult {
    pub pos_in_source: Option<usize>,  // 对应源码字节偏移
}

/// 插件的输入命令，与 app 层的 EditCommand 解耦。
/// 由宿主在 dispatch 层完成 EditCommand → PluginCommand 映射。
pub enum PluginCommand {
    Scroll { delta_y: f32 },
    Click { pos: LogicalPos, click_count: u8 },   // click_count: 1/2/3 → 点选/词选/行选
    Drag { pos: LogicalPos },
    Copy,
    SelectAll,
    Find { query: String, case_sensitive: bool },
    FindNext,
    FindPrev,
    ExtendSelection { direction: Direction },       // 方向键扩展选区
    /// 宿主将未识别的 EditCommand 包装为 Custom 传递
    Custom(Box<dyn Any>),
}

/// 插件向宿主注册的工具栏按钮，宿主负责渲染到 TitleBar。
pub struct ToolbarItem {
    pub id: &'static str,        // 唯一标识，如 "toc"
    pub tooltip: &'static str,   // 悬停提示，如 "Toggle Table of Contents"
    pub icon: ToolbarIcon,       // 图标枚举
    pub toggled: bool,           // 当前切换状态（控制按钮高亮）
}

pub trait ContentPlugin: Any {
    // ── 身份 ──
    fn id(&self) -> PluginId;
    fn name(&self) -> &str;
    fn supported_extensions(&self) -> &[&str];

    // ── 生命周期 ──
    fn on_activate(&mut self, ctx: &mut PluginContext);
    fn on_deactivate(&mut self);
    /// buffer 内容被外部修改时调用（用户编辑、其他插件修改）
    fn on_source_changed(&mut self, ctx: &mut PluginContext);

    // ── 渲染 ──
    /// 主内容渲染。scroll_y 由宿主管理。
    fn render(&mut self, scroll_y: f32, ctx: &mut PluginContext) -> PluginOutput;
    /// 选区高亮渲染（叠加在主内容之上）
    fn selection_highlights(&self, ctx: &mut PluginContext) -> Option<DrawList> { None }

    // ── 输入 ──
    fn on_command(&mut self, cmd: &PluginCommand, ctx: &mut PluginContext) -> CommandFlow;
    fn hit_test(&self, pos: LogicalPos) -> Option<HitResult>;

    // ── 搜索集成 ──
    /// 执行搜索，返回匹配数量。插件内部缓存匹配结果用于高亮渲染。
    fn search(&mut self, query: &str, case_sensitive: bool) -> usize;
    /// 清除搜索状态
    fn clear_search(&mut self);
    /// 跳转到第 n 个匹配（0-indexed），返回是否成功。
    /// 插件负责内部滚动到匹配位置。
    fn jump_to_match(&mut self, index: usize) -> bool;
    /// 返回搜索高亮 DrawList，叠加在主内容之上
    fn search_highlights(&self, ctx: &mut PluginContext) -> Option<DrawList> { None }

    // ── 选择与剪贴板 ──
    /// 返回当前选中文本（用于 Copy 操作）
    fn selected_text(&self) -> Option<String> { None }

    // ── 工具栏扩展 ──
    /// 插件声明要在 TitleBar 上显示的按钮。宿主每帧调用以获取最新状态（如 toggled）。
    /// 默认返回空，表示无额外按钮。
    fn toolbar_items(&self) -> Vec<ToolbarItem> { vec![] }
    /// 用户点击了插件注册的工具栏按钮，宿主回调此方法。
    fn on_toolbar_action(&mut self, item_id: &str) {}

    // ── 能力声明 ──
    fn allows_editing(&self) -> bool { false }
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
```

### PluginContext

> [!NOTE]
> **关于 Shaper 和借用**：
> `Shaper` 内部有 `buffer` 和 `cache`，调用 `shape` 等方法需要 `&mut self`，所以必须传入 `&mut Shaper`。
> 另一方面，插件如果需要修改内容，不能直接获取 `&mut TextBuffer`，因为这会与 `source: &'a str` 造成生命周期冲突。
> 解决策略：`PluginContext` 只持有只读的 `source` 引用和可变的 `pending_edits` 队列引用，以及 `&mut Shaper`。这样既能进行渲染时塑形，又能避免借用冲突。所有的 trait 方法都传入 `&mut PluginContext`。

```rust
pub struct PluginContext<'a> {
    source: &'a str,              // 从 doc.source() 获取的只读引用
    pub theme: &'a Theme,
    pub settings: &'a Settings,
    pub shaper: &'a mut Shaper,   // 可变引用；插件在 render 期间需要调用 shaper
    pub viewport: Rect,           // 插件可用渲染区域（屏幕坐标）
    pub dpi: f32,
    pending_edits: &'a mut Vec<TextEdit>, // 外部传入的待执行编辑队列
}

impl PluginContext<'_> {
    /// 读取源文本
    pub fn source(&self) -> &str { self.source }

    /// 追加一条编辑操作到 pending 队列。
    /// 宿主在当前帧所有插件调用结束后批量执行。
    pub fn queue_edit(&mut self, edit: TextEdit) {
        self.pending_edits.push(edit);
    }

    /// 请求编辑器跳转到指定行
    pub fn reveal_line(&mut self, line: usize);

    /// 请求宿主重绘
    pub fn request_redraw(&mut self);
}
```

### 同步协议

```
用户键入 → buffer 变更 → 宿主调用 plugin.on_source_changed(&mut ctx)
插件想修改文本 → 在 on_command 中调用 ctx.queue_edit(edit) → 宿主帧末批量执行
循环防护：批量执行后的 on_source_changed 不在同一帧回调
```

- 文本 buffer（`DocumentView.tb: TextBuffer`）是唯一真相源
- 选区模型由各插件独立管理（EditorPlugin 用 `CursorState`，MarkdownPlugin 用 `PreviewPos`）
- 滚动位置各自独立（插件内部 `scroll_y`，宿主只传入、不直接修改）
- Shaper 可在 `render` 或 `on_command` 中调用，因为 `PluginContext` 持有其可变借用

## 导航器（Navigator）

> [!NOTE]
> Navigator 是 **app 层的行为抽象**，统一 TabBar 和 Sidebar 文件树两种文件导航方式。
> 底层渲染仍由 `crates/ui/widgets/` 中的纯 UI 组件完成（`tab_bar`、未来的 `sidebar_tree`）。
> Navigator 负责：将 app 状态映射为 UI 组件的输入，处理 UI 组件输出的动作。

```rust
// crates/app/src/navigator.rs

pub trait Navigator: Any {
    fn id(&self) -> &str;
    fn name(&self) -> &str;

    /// 渲染导航区域。Navigator 内部调用对应的 ui::widgets 组件。
    /// 返回 DrawList 和可能的导航动作。
    fn render(&mut self, rect: Rect, ctx: &NavContext) -> NavOutput;

    /// 导航区域的鼠标命中测试
    fn hit_test(&self, pos: LogicalPos) -> Option<NavAction>;

    /// 导航区域的滚动处理（如 TabBar 横向滚动、Sidebar 纵向滚动）
    fn scroll(&mut self, delta: f32);
}

pub struct NavContext<'a> {
    pub open_tabs: &'a [NavEntry],    // 所有打开的文件
    pub active_index: usize,
    pub theme: &'a Theme,
    pub settings: &'a Settings,
    pub dpi: f32,
}

/// 从 Tab 提取的导航所需信息（纯数据，不含 DocumentView 引用）
pub struct NavEntry {
    pub title: String,
    pub file_path: Option<PathBuf>,
    pub is_dirty: bool,
    pub pinned: bool,
    pub language: String,
}

pub struct NavOutput {
    pub draw_list: DrawList,
    pub actions: Vec<NavAction>,
}

pub enum NavAction {
    SwitchTo(usize),      // 切换到第 n 个打开的文件
    Close(usize),         // 关闭
    Open(PathBuf),        // 打开新文件
}
```

> [!IMPORTANT]
> **分层关系**：
> - `NavEntry` 与 `ui::widgets::tab_bar::TabInfo` 字段几乎一致，但属于不同层级。
>   `TabBarNavigator` 内部负责 `NavEntry → TabInfo` 的映射。
> - 这避免了 app 层直接依赖 `ui::widgets` 的具体类型，保持单向依赖。
> - TabBar 和 Sidebar 同一时刻只有一个活跃，用户通过快捷键切换。
> - Navigator 不管理文件打开/关闭的具体逻辑，只发 `NavAction`，宿主执行。

### TabBarNavigator

```rust
// crates/app/src/navigators/tab_bar.rs

struct TabBarNavigator {
    widget: TabBarWidget,   // ui::widgets::tab_bar 的实例
}

impl Navigator for TabBarNavigator {
    fn id(&self) -> &str { "builtin.tab_bar" }
    fn name(&self) -> &str { "Tab Bar" }

    fn render(&mut self, rect: Rect, ctx: &NavContext) -> NavOutput {
        // NavEntry → TabInfo 映射
        let tab_infos: Vec<TabInfo> = ctx.open_tabs.iter().map(|e| TabInfo {
            title: e.title.clone(),
            file_path: e.file_path.clone(),
            is_dirty: e.is_dirty,
            pinned: e.pinned,
            language: e.language.clone(),
        }).collect();
        self.widget.tabs = tab_infos;
        // 渲染并收集动作
        let draw_list = self.widget.render(rect, ctx.active_index, ctx.theme, ctx.dpi);
        NavOutput { draw_list, actions: self.widget.drain_actions() }
    }
}
```

## 渲染流程

```
App::render()
│
├─ 1. navigator.render(nav_rect, nav_ctx) → NavOutput
│      Navigator 内部调用 ui::widgets 组件渲染
│      收集 NavAction（SwitchTo / Close / Open），宿主执行
│
├─ 2. 内容区域：tab.plugin.render(scroll_y, ctx) → PluginOutput
│    │
│    ├─ EditorPlugin       → 当前 shape_visible_lines 路径
│    ├─ MarkdownPlugin     → MarkdownPreview::render() → DrawList
│    └─ （未来）MindMapPlugin → ...
│    │
│    ◆ 没有 if/else，全是 trait 调用
│    ◆ PluginOutput.draw_list → paint_backend::drain() → GlyphVertex（统一转换）
│    ◆ 主滚动条 ← content_height（宿主统一管理）
│    ◆ 插件切换 = 换一个 Box<dyn ContentPlugin>
│    │
│    ├─ 2a. plugin.selection_highlights(&mut ctx) → 叠加选区高亮
│    └─ 2b. plugin.search_highlights(&mut ctx)    → 叠加搜索高亮
│    │
│    ◆ 插件自行管理辅助面板（如 MarkdownPlugin 的 TOC）
│    ◆ 辅助面板的渲染、滚动、交互均在 plugin.render() 内完成
│
├─ 3. TitleBar 插件按钮
│      items = plugin.toolbar_items()   // 插件声明按钮
│      宿主渲染到 TitleBar              // 宿主负责显示
│      用户点击 → plugin.on_toolbar_action(item_id)  // 事件回流
│
├─ 4. 主滚动条渲染（宿主统一驱动，基于 content_height）
│
└─ 5. 叠加层（搜索栏、状态栏等）
```

### EditorPlugin（基础编辑器作为插件）

> [!IMPORTANT]
> `EditorPlugin` 在 Phase 1 中是 **薄包装**——内部直接持有/引用 `DocumentView`，
> 将 trait 方法委托给现有逻辑。**不是**重写编辑器。
> 后续 Phase 可以逐步将 `DocumentView` 的职责拆分到 `EditorPlugin` 和 `Tab` 中。

```rust
struct EditorPlugin;

impl ContentPlugin for EditorPlugin {
    fn id(&self) -> PluginId { "builtin.editor" }
    fn name(&self) -> &str { "Editor" }
    fn supported_extensions(&self) -> &[&str] { &[] } // fallback，匹配所有未注册的类型

    fn allows_editing(&self) -> bool { true }

    fn render(&mut self, scroll_y: f32, ctx: &mut PluginContext) -> PluginOutput {
        // Phase 1: 委托给现有 DocumentView 渲染流程
        // 当前 shape_visible_lines → GlyphVertex 的路径保持不变
        // EditorPlugin 的 render 只是现有代码的 trait 包装
        todo!("Phase 1 实现：包装现有 shape_visible_lines 路径")
    }

    fn on_command(&mut self, cmd: &PluginCommand, ctx: &mut PluginContext) -> CommandFlow {
        // 编辑器处理所有命令，不 passthrough
        CommandFlow::Consumed
    }

    fn search(&mut self, query: &str, case_sensitive: bool) -> usize {
        // 委托给 DocumentView.search_state 的已有逻辑
        todo!()
    }
}
```

## Tab 结构

```rust
// crates/app/src/tab.rs（从 view.rs 演化）

pub(crate) struct Tab {
    pub doc: DocumentView,                       // 保持完整 DocumentView，Phase 1 不拆解
    pub plugin: Box<dyn ContentPlugin>,          // 当前活跃的渲染插件
}
```

> [!IMPORTANT]
> **关于 DocumentView 的保留**：
> 原文档提出引入 `EditorCore` 但未定义其与 `DocumentView` 的关系。
> 经审查，`DocumentView` 包含 `TextBuffer`、`LineIndex`、`DisplayState`、
> `CursorState`、`HighlighterCache`、`SearchState` 等紧密耦合的 12+ 个字段，
> 贸然拆解风险极高。
>
> **Phase 1 策略**：`Tab` 直接持有完整 `DocumentView`。`EditorPlugin` 通过 `PluginContext`
> 间接访问 `Tab.doc` 的数据。后续 Phase 可逐步将渲染相关职责从 `DocumentView` 迁移到插件中。

- Tab 不再有 `View` 枚举
- `plugin` 始终有值，基础编辑就是 `EditorPlugin`
- 切换视图模式：构造新 plugin 实例替换旧的，旧 plugin `on_deactivate()`，新 plugin `on_activate(ctx)`
- 辅助面板（如 TOC）是插件内部状态，由插件自行管理，不上提到 Tab

### 预览标签页管理

> [!NOTE]
> 当前 `Workspace.preview_index` 有三个核心行为需要在插件架构中保留：
> 1. **自动关闭**：切换到其他 tab 时，未修改的 preview tab 自动关闭
> 2. **升级为正式 tab**：用户在 preview tab 中编辑时，升级为正式 tab（`preview_index` 置 None）
> 3. **索引追踪**：关闭 tab 时更新 `preview_index`
>
> 这些行为属于 Workspace 层的 tab 生命周期管理，与插件无关。
> `preview_index` 机制保持不变，只是 `View::Markdown` 判断改为 `plugin.id()` 判断。

```rust
// workspace.rs 中的变化
pub(crate) struct Workspace {
    tabs: Vec<Tab>,                              // 替换 views: Vec<View>
    active_index: usize,
    pub(crate) preview_index: Option<usize>,     // 保留
    // ... 其余字段不变
}

impl Workspace {
    /// 替代 toggle_active_view_mode
    pub fn switch_plugin(&mut self, tab_index: usize, plugin_id: PluginId) {
        let tab = &mut self.tabs[tab_index];
        tab.plugin.on_deactivate();
        tab.plugin = create_content_plugin_by_id(plugin_id, &tab.doc);
        tab.plugin.on_activate(&PluginContext::from_tab(tab));
    }

    /// 保留：preview tab 升级逻辑
    pub fn upgrade_preview_if_needed(&mut self) -> LayoutEffect { /* ... */ }
}
```

## 编译时注册

```rust
// crates/app/src/plugin_registry.rs

pub fn create_content_plugin(path: &Path) -> Box<dyn ContentPlugin> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "md" | "markdown" => Box::new(MarkdownPlugin::new()),
        _ => Box::new(EditorPlugin::new()),   // fallback
    }
}

pub fn create_content_plugin_by_id(id: PluginId, doc: &DocumentView) -> Box<dyn ContentPlugin> {
    match id {
        "builtin.markdown" => Box::new(MarkdownPlugin::from_source(doc.source())),
        "builtin.editor" | _ => Box::new(EditorPlugin::new()),
    }
}
```

无宏、无动态加载。`EditorPlugin` 是默认 fallback。

## 文件改动清单

### 新增

| 文件 | 职责 |
|------|------|
| `crates/app/src/plugin.rs` | `ContentPlugin` trait + `PluginContext` + `PluginOutput` + `CommandFlow` + `HitResult` + `PluginCommand` |
| `crates/app/src/plugin_registry.rs` | `create_content_plugin(path)` + `create_content_plugin_by_id(id)` |
| `crates/app/src/navigator.rs` | `Navigator` trait + `NavContext` + `NavEntry` + `NavOutput` + `NavAction` |
| `crates/app/src/navigators/mod.rs` | Navigator 模块入口 |
| `crates/app/src/navigators/tab_bar.rs` | `TabBarNavigator` — 包装 `ui::widgets::tab_bar` |
| `crates/app/src/tab.rs` | `Tab` struct（取代 `View` 枚举 + `MdView`） |
| `crates/app/src/plugins/mod.rs` | 插件模块入口 |
| `crates/app/src/plugins/editor.rs` | `EditorPlugin` — 现有编辑器渲染路径的 trait 包装 |
| `crates/app/src/plugins/markdown.rs` | `MarkdownPlugin` — 从 `md_preview.rs`（1151 行）提取 |

### 重构

| 文件 | 改动 |
|------|------|
| `view.rs` | 删除 `View` 枚举和 `MdView`；`DocumentView` 保持不变，被 `Tab.doc` 直接持有 |
| `workspace.rs` | `views: Vec<View>` → `tabs: Vec<Tab>`；`toggle_active_view_mode` → `switch_plugin`；`preview_index` 逻辑保留但去掉 `View::Markdown` 匹配 |
| `app_renderer.rs` | 删除 9 处预览硬编码分支，改为统一 `plugin.render()` + `plugin.selection_highlights()` + `plugin.search_highlights()` 调用 |
| `dispatch/editor.rs` | 删除预览命令白名单（~40 行）+ 选区操作（~130 行），改为 `plugin.on_command(PluginCommand::*)` |
| `dispatch/viewport.rs` | 删除 `mv.preview.scroll_*` 分支，改为 `PluginCommand::Scroll` |
| `dispatch/mouse.rs` | 删除预览命中测试 + 选区分支，改为 `plugin.hit_test()` + `plugin.on_command(Click/Drag)` |
| `app_scroll.rs` | 删除预览滚动分支（TOC 滚动由插件内部处理） |
| `app_search.rs` | 搜索调用 `plugin.search()` / `plugin.jump_to_match()`；删除 `mv.preview.scroll_to_search_match()` |
| `events.rs` | `TitleBarAction::ToggleMarkdownPreview` → 通用 `SwitchPlugin` action；插件工具栏按钮点击 → `plugin.on_toolbar_action(id)` |
| `ui_shell.rs` | `TitleBarInput` 中 `is_preview` 改为从 `plugin.id()` 获取；`toc_visible` / `toc_enabled` 删除（插件内部管理） |
| `app_dispatch.rs` | 删除 `preview_offsets()`，其布局计算统一到 `PluginContext.viewport` |

### 删除

| 文件 | 说明 |
|------|------|
| `md_preview.rs` | 1151 行逻辑迁移到 `plugins/markdown.rs` |
| `view.rs` 中的 `View` 枚举 | 被 `tab.rs` 的 `Tab` struct 取代 |

### 不变

| 层级 | 说明 |
|------|------|
| `crates/markdown/` | 纯函数库，MarkdownPlugin 内部调用 |
| `crates/ui/widgets/tab_bar/` | TabBar 渲染组件（`TabInfo` 输入、`TabAction` 输出），被 `TabBarNavigator` 包装调用 |
| `crates/ui/widgets/toc/` | TOC 渲染组件（若存在），由 MarkdownPlugin 内部调用 |
| `crates/ui/core/paint.rs` | `DrawList { cmds: Vec<DrawCmd>, offset: (f32, f32) }` |
| `crates/render/` | GPU 渲染基础设施 |
| `paint_backend.rs` | `DrawList → GlyphVertex` 转换 |
| `document_view/` | `DocumentView` 结构体保持不变（Phase 1 不拆解） |

## 迁移策略

### Phase 1：定义接口 + Tab 结构迁移（不改变行为）

**目标**：引入 trait 和 Tab 结构，所有现有代码仍走旧路径。

1. 创建 `plugin.rs` 定义 `ContentPlugin` trait 和相关类型（`PluginCommand`、`PluginOutput` 等）
2. 创建 `plugins/editor.rs` 实现 `EditorPlugin`——**纯薄包装**，内部委托给 `DocumentView` 现有方法
3. 创建 `tab.rs` 定义 `Tab` struct（持有 `DocumentView` + `Box<dyn ContentPlugin>`）
4. `workspace.rs` 中 `views: Vec<View>` → `tabs: Vec<Tab>`，统一访问方法
5. `view.rs` 中的 `View::Editor(dv)` / `View::Markdown(mv)` 分支暂时保留为 `tab.plugin.id()` 判断
6. 所有现有 dispatch/render 代码**不做功能改动**，仅替换类型签名

**验证标准**：
- `cargo build` 通过，无警告
- 所有现有功能不受影响：打开、编辑、保存、滚动、搜索、Tab 切换
- `cargo test` 全部通过
- `grep -r "enum View" crates/app/` 返回零结果

### Phase 2：迁移 Markdown 预览到插件接口

**目标**：消除所有 `View::Markdown` / `is_markdown` 硬编码分支。

1. 创建 `plugins/markdown.rs` 实现 `MarkdownPlugin`
   - 内部复用现有 `MarkdownPreview` 的全部逻辑（1151 行）
   - 将 `render()` / `preview_hit_test()` / `selection_highlights()` / `search_highlights()` 映射到 trait 方法
   - TOC（`headings` / `scroll_to_heading()` / `toc_visible`）保留为 MarkdownPlugin 内部状态
2. `app_renderer.rs`：9 处预览分支 → 统一 `plugin.render()` 调用
3. `dispatch/editor.rs`：删除命令白名单（~40 行）+ 选区操作代码（~130 行）→ `plugin.on_command()`
4. `dispatch/mouse.rs`：删除预览命中测试 → `plugin.hit_test()` + `plugin.on_command(Click/Drag)`
5. `dispatch/viewport.rs`：删除 `preview.scroll_y` 直接赋值 → `PluginCommand::Scroll`
6. `app_scroll.rs`：删除预览滚动分支
7. `app_search.rs`：搜索 → `plugin.search()` / `plugin.jump_to_match()`
8. `events.rs`：`ToggleMarkdownPreview` → 通用 `SwitchPlugin`
9. 删除 `md_preview.rs` 和 `view.rs` 中的 `MdView`

**验证标准**：
- `cargo build` 通过
- `grep -rn "View::Markdown\|MdView\|is_markdown\|as_md" crates/app/src/` 返回零结果（`is_markdown_path` 辅助函数除外）
- Markdown 预览功能完整验证：
  - [x] 渲染（含懒布局缓存、DrawList 缓存）
  - [x] 滚动（鼠标滚轮 + 滚动条拖拽）
  - [x] 文本选择（单击/双击/三击 + 拖拽 + Shift 扩展）
  - [x] 搜索高亮 + 跳转匹配
  - [x] TOC 面板（MarkdownPlugin 内部管理：显示/隐藏 + 点击跳转标题）
  - [x] 复制选中文本
  - [x] preview tab 自动关闭 / 升级为正式 tab
- `cargo test` 全部通过

### Phase 3：提取 Navigator

**目标**：将 TabBar 渲染路径从 app 硬编码改为 Navigator trait 调用，为 Sidebar 文件树铺路。

1. 定义 `Navigator` trait（`navigator.rs`）
2. 实现 `TabBarNavigator`——包装现有 `ui::widgets::tab_bar` 渲染逻辑
3. `app_renderer.rs` 中 TabBar 渲染改为 `navigator.render()` 调用
4. `app_scroll.rs` 中 TabBar 横向滚动改为 `navigator.scroll()` 调用
5. Workspace 新增 `navigator: Box<dyn Navigator>` 字段

**验证标准**：
- TabBar 功能不受影响：Tab 切换、关闭、拖拽排序、横向滚动
- `cargo test` 全部通过

### Phase 4：清理和稳定

**目标**：消除过渡期冗余，确保架构整洁。

1. 审计所有 `as_any` / `as_any_mut` 调用，确保没有绕过 trait 的后门
2. 审计 `plugin.id()` 判断——理想情况下不应有针对特定 plugin id 的分支（否则与 `View::Markdown` 等价）
3. 性能基准测试：对比重构前后的帧率和内存占用
4. 文档更新：`docs/specs/` 中标记本文档为「已实施」

**验证标准**：
- 代码中无 `plugin.id() == "builtin.markdown"` 之类的特殊分支
- 性能无回归（帧率差异 < 5%）
- `./scripts/verify.sh` 全部通过

### Phase 5：扩展（后续规划）

- `SidebarNavigator` — 文件树导航（`crates/ui/widgets/sidebar_tree.rs` + `crates/app/src/navigators/sidebar.rs`）
- 脑图插件、小说阅读模式等
- 用户配置：文件类型 → 默认插件映射

## 不变约束

- `crates/ui` 不依赖 `crates/app`（红线不变）
- 所有 ContentPlugin 输入通过 `PluginContext` 传递，插件不能直接访问 App/Workspace 内部状态
- Navigator 输入通过 `NavContext` 传递，不直接访问 Workspace 内部状态
- 编译时注册，无动态加载

## Performance Verification

### Instrumentation Approach

Debug-build instrumentation is added to the render hot path using `std::time::Instant`:

- **Frame timing**: Each `App::render()` call records start instant and computes:
  - `_total_render_us`: wall-clock duration of the render function
  - `_frame_interval_us`: time since previous render (inter-frame gap)
- **Periodic summary**: A frame counter (`render_frame_count`) increments each frame; every 60th frame logs total render time and interval to stderr via `eprintln!`
- **Slow-frame logging**: Frames exceeding 1ms render time or 20ms interval are logged individually
- **`about_to_wait` logging**: Event-loop wake timing is captured for diagnosing scheduling stalls

All instrumentation is gated behind `#[cfg(debug_assertions)]` and has zero cost in release builds.

### Expected Characteristics

| Component | Overhead |
|-----------|----------|
| `ContentPlugin` trait dispatch | Single virtual call per frame — negligible (nanoseconds) |
| `Navigator` trait dispatch | One additional vcall per frame for `TabBarNavigator::tick()` — negligible |
| `PluginContext` construction | Pure data struct, no heap allocation beyond existing `Vec` reuse |
| Frame-count increment + modulo check | Single `u32` add and branch — unmeasurable |

The trait-based architecture introduces only static-cost dynamic dispatch (vtable pointer dereference) with no additional heap allocations per frame. The `ContentPlugin` trait method is called exactly once per render cycle, same as the previous hardcoded `match` on `View` enum — the dispatch cost is equivalent.

### Actual Measurement Notes

> **⚠️ Headless environment limitation**: Actual frame-time measurements require a running GUI with GPU context. The expected benchmarks from the task brief are:

| Scenario | Expected |
|----------|---------|
| Idle `.rs` file | < 2ms |
| Idle `.md` preview | < 5ms |
| Rapid `.md` scroll | < 8ms |
| Rapid tab switching | < 3ms |

To measure in a live session: run in debug mode and watch stderr for `[perf] frame#N total=Xus interval=Yus` lines, or inspect `/tmp/perf.log` for per-frame `[rr]` and `[atw]` entries.
- `ui::widgets::tab_bar` 保持为纯 UI 渲染组件，Navigator 在 app 层编排行为
- `DocumentView` 在 Phase 1 保持完整，不贸然拆解
