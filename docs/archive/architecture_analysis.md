# Edit+ 架构与全链路分析报告

根据对工程代码库的全面分析（包含依赖分析、静态检查及代码结构审查），总结出以下几个核心可以优化的架构层面问题，以及大量冗余、已弃用的代码。

工程正处于从 **Phase 1 (Immediate Mode / 数据驱动 UI)** 向量 **Phase 6+ (OOP Widget Tree 架构)** 演进的中间状态。这导致了目前最大的结构性冗余。

## 1. 最大的结构冗余：两套 UI 体系并存

目前 `crates/ui` 下存在严重的代码组织冗余，旧有的状态函数实现与新的 Widget 包装层并存。

### Sidebar 的多重计算 (性能隐患)
- `crates/ui/src/sidebar.rs` (旧状态机与布局) vs `crates/ui/src/widgets/sidebar.rs` (新 Widget 封装)。
- 在 `widgets/sidebar.rs` 中，侧边栏文件列表的绘制和点击命中（Hit Test）已经被全权委托给了底层的 `VerticalListWidget`。
- **严重冗余**：但在 `SidebarWidget::set_rect` 期间，依然会调用旧版的 `self.state.update_layout(&input, &self.cfg)`。该方法会在底层反复遍历 Tabs，并生成 `items: Vec<SidebarLayoutItem>`。这段计算结果不会被用于渲染，也完全失去了存在的意义。
- **内存分配开销**：在 `SidebarWidget::set_rect`（几乎每帧调用或每次布局调用）中，有以下代码：
  ```rust
  let items: Vec<ListItem> = self.tabs.iter().map(|t| ListItem {
      label: t.title.clone(), // ⬅️ 灾难性的每帧字符串克隆
      //...
  ```
  这会引发高频的无用内存分配，是极佳的性能优化点。

### TabBar 与 Scrollbar 的包装
- `tab_bar/mod.rs`（和其子模块 `state.rs`, `layout.rs` 等）处理着真正的逻辑，然后由 `widgets/tab_bar.rs` 进行了浅层包装。
- 对于 `scrollbar` 和 `status_bar`，旧的逻辑已经被成功剥离成纯文本/坐标计算器，由 Widget 纯粹处理渲染。这是好的，但目录结构上依然是散落的（如 `src/scrollbar.rs` 与 `src/widgets/scrollbar.rs` 分离），可以合并收编到各自的模块中。

### 丑陋的跨帧状态转移
- `SidebarWidget` 存在 `steal_state()`, `inject_state()`, `steal_persistent()` 等方法，用来在 App 帧间反复传递状态。这打破了 Widget 树应有的状态持久化封装。如果 Widget 架构成熟，这部分 State 应该原生托管在 Widget 树节点中。

## 2. Widget 架构不完善 (全链路派发问题)

虽然 `crates/ui` 定义了 `Widget`, `WidgetId`, `EventCtx`, `LayoutCtx`，但在 `crates/app` 中并没有形成一颗统一自动化管理的树。
- `crates/app/src/ui_shell.rs` **手动管理和编排**了 `SidebarWidget`, `TabBarWidget`, `SearchBarWidget`。
- 事件分发在 `events.rs` 和 `ui_shell.rs` 中是硬编码调用的（例如：如果点击了侧边栏则调 A，如果点击了状态栏则调 B）。
- **优化建议**：`UiShell` 应蜕变为一个根 `ContainerWidget`（例如通过 `Dock` 直接管控），所有输入（`LayoutCtx`, `PaintCtx`）通过 `widget.paint(ctx)` 递归自动分发，彻底消灭 `ui_shell.rs` 中一堆冗余的 `set_xxx_input` 胶水代码。

## 3. 死代码与闲置结构 (Dead Code)

通过静态分析，找出了以下完全不用或多余的代码：

**废弃方法/实现：**
- `crates/app/src/app_renderer.rs`: `render_text_fragments` 已经被弃用。
- `crates/app/src/sys/macos_titlebar.rs`: `traffic_light_inset` 函数从未使用。
- `crates/app/src/ui_shell.rs`: `rebuild_and_layout` 方法从未使用。
- `crates/app/src/actions.rs`: `AppAction::ScrollbarAction` 从未被构造。
- 常量 `WINDOW_TITLE` 在 `app_lifecycle.rs` 里闲置。

**无用引用与变量：**
- `app_renderer.rs` 中的 `ui::tab_bar`，以及局部变量 `show_tabs`, `tbh`, `line_height`。
- `document_view/mod.rs` 中引入了 `DisplayLineMap` 和 `RenderCache` 但没有使用，说明显示缓存的逻辑发生过重构，原有的引入残留了。
- `paint_backend.rs` 中 `is_whitespace_cluster` 引入未用。
- `app.rs` 和 `events.rs` 中的各种屏幕宽高局部变量 (`screen_w`, `screen_h`) 被多处计算后直接抛弃。
- `reshape_worker.rs` 里的 `proxy` 被赋值但没读取。

**可见性泄露：**
- `crates/app/src/document_view/mod.rs` 中 `display` 字段被声明为 `pub`，但返回了私有的 `DisplayState`。
- `cursor` 方法同样返回了被限制私有访问的 `CursorState`。
- 建议将 `DisplayState` 和 `CursorState` 修改为 `pub`，或将暴漏它们的接口设为 `pub(crate)`。

**无意义的 Copy Drop：**
- `app_renderer.rs` `222` 行：`drop(lc)`，`lc` 是 `usize` (Copy 类型)，`drop` 调用会被编译器忽略且无任何释放内存的作用。

## 4. 全链路优化实施路线建议

1. **清理阶段 (Phase A)**: 
   - 彻底删除 `cargo check` 报出的 unused warning。修复隐私限制 (Visibility) 问题。
   - 删除 `SidebarState` 内部所有关于 `items` 的计算逻辑，移除 `hit_test_px` 里关于文件列表项的判定，因为这些职责已经物理移交到了 `VerticalListWidget`。
2. **重构阶段 (Phase B)**:
   - 解决 `SidebarWidget` 每帧生成 `Vec<ListItem>` 时 `title.clone()` 带来的内存分配问题（建议改成传入 `Cow` 或者通过引用比较变化来脏检查）。
   - 把分散的 `ui/src/sidebar.rs` 和 `ui/src/widgets/sidebar.rs` 整合到一个 `ui/src/sidebar/` 模块下。
3. **彻底贯彻 Widget 树 (Phase C)**:
   - 将 `UiShell` 中大量平铺的数据结构收拢，将硬编码事件派发重构为标准的 UI Tree 递归派发。
