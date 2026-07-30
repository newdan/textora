# UI 骨架架构审计报告

**审计范围**: `crates/ui/` (16 文件 + 3 子目录) + `crates/app/` 的 UI 集成层  
**审计时段**: 2025-06-11 以来的提交  
**审计日期**: 2025-06-12

---

## 项目现状概览

### UI Crate 结构

```
crates/ui/src/
├── core/                 ← UI 框架原语层 (Widget trait, Dock, DrawCmd, Rect)
│   ├── geom.rs           (4.1K — Rect, Screen, NDC)
│   ├── paint.rs          (5.3K — DrawCmd, DrawList)
│   ├── measure.rs        (904B — TextMeasure trait)
│   ├── widget.rs         (7.6K — Widget trait, LayoutCtx, PaintCtx, EventCtx)
│   └── dock.rs           (19.4K — Dock 布局容器)
├── widgets/              ← Widget trait 实现层
│   ├── scrollbar.rs      (18.4K)
│   ├── search_bar.rs     (16.2K)
│   ├── sidebar.rs        (39.6K)
│   ├── status_bar.rs     (11.5K)
│   ├── tab_bar.rs        (5.0K)
│   ├── list.rs           (14.8K)
│   └── popup_menu.rs     (8.2K)
├── tab_bar/              ← 旧 tab bar（独立模块，8 文件）
│   ├── layout.rs, state.rs, render.rs, hit.rs, text.rs, types.rs, tests.rs
├── sidebar.rs            (41.6K — SidebarConfig/SidebarState, 与 widgets/sidebar.rs 并存)
├── popup_menu.rs         (31.8K — NDC+px API 并存)
├── scrollbar.rs          (3.3K — 旧版)
├── status_bar.rs         (5.1K — StatusBarInput)
├── search_bar.rs         (377B — 仅剩 SEARCH_BAR_HEIGHT)
├── viewport.rs           (35K)
├── layout.rs             (24.3K)
├── settings.rs           (13K)
├── theme.rs              (14.3K)
├── decorations.rs        (6.8K)
├── gutter.rs             (7.1K)
├── title_bar.rs          (5.5K)
├── text_renderer.rs      (673B)
└── view_mode.rs          (386B)
```

### App Crate 关键文件

| 文件 | 大小 | 职责 |
|------|------|------|
| [app.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/app.rs) | **151KB** | App 主体、生命周期、winit handler |
| [app_renderer.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/app_renderer.rs) | 32KB | 顶点生成、GPU 提交 |
| [render_pipeline.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/render_pipeline.rs) | 52KB | shaping 主循环 |
| [ui_shell.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/ui_shell.rs) | 26KB | Dock + Widget 编排 |
| [commands.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/commands.rs) | 63KB | 编辑命令 |
| [workspace.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/workspace.rs) | 40KB | 多标签工作区 |
| [document_view/](file:///Users/dan/proj/llmws/edit+/crates/app/src/document_view) | 52KB+ | DocumentView 的 8 个子模块 |
| [events.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/events.rs) | 21KB | 鼠标/键盘事件分发 |

---

## 一、架构优化建议

### 1.1 🔴 新旧两套渲染路径并存 — 幽灵代码

> [!WARNING]
> 这是当前架构的最大技术债务。新的 Widget/DrawCmd 体系已经搭建，但旧的直接顶点生成代码仍然存在并被使用。

**现状**: 以下组件存在**新旧两套实现并存**：

| 组件 | 旧代码 | 新代码 | 状态 |
|------|--------|--------|------|
| Scrollbar | [scrollbar.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/scrollbar.rs) (3.3K) | [widgets/scrollbar.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/widgets/scrollbar.rs) (18.4K) | 并存 |
| StatusBar | [status_bar.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/status_bar.rs) (5.1K) | [widgets/status_bar.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/widgets/status_bar.rs) (11.5K) | 并存 |
| SearchBar | [search_bar.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/search_bar.rs) (377B, 仅常量) | [widgets/search_bar.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/widgets/search_bar.rs) (16.2K) | 基本完成 |
| Sidebar | [sidebar.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/sidebar.rs) (41.6K) | [widgets/sidebar.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/widgets/sidebar.rs) (39.6K) | **81K** 重复 |
| TabBar | [tab_bar/](file:///Users/dan/proj/llmws/edit+/crates/ui/src/tab_bar) (53K, 8 文件) | [widgets/tab_bar.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/widgets/tab_bar.rs) (5K) | 旧版大量存在 |
| PopupMenu | [popup_menu.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/popup_menu.rs) (31.8K, NDC API) | [widgets/popup_menu.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/widgets/popup_menu.rs) (8.2K) | NDC API 待删 |

**问题**:
- `sidebar.rs` 和 `widgets/sidebar.rs` 合计 **81K**，两份几乎等量代码
- `tab_bar/` 目录 53K 旧代码 vs `widgets/tab_bar.rs` 5K 新适配，旧代码仍被 app 层直接引用
- `popup_menu.rs` 注释提到 "Phase 8 末尾删除旧 NDC API"，但 Phase 8 看起来已经过去
- `app_renderer.rs` 中存在重复的 `tab_infos` 构造（L348 和 L400），暗示新旧两套流程并行

**建议**: 制定迁移清理计划，按优先级删除旧代码：
1. `search_bar.rs` → 仅剩常量，可内联到 `widgets/search_bar.rs`
2. `popup_menu.rs` 的 NDC API → 标记为 `#[deprecated]` 并在下一迭代移除
3. `sidebar.rs` vs `widgets/sidebar.rs` → 明确哪个是 source of truth
4. `tab_bar/` → 逐步将 state/layout 的核心逻辑迁入 widget 体系

---

### 1.2 🔴 `app.rs` 巨型文件（151KB / ~3600 行）

**问题**: 一个文件承载了：
- App struct 定义和初始化
- `ApplicationHandler` trait 实现
- `init_display_map()`
- 工作区管理逻辑
- 多处渲染相关调用

**影响**:
- 难以 review 和维护
- 编译时间增长（单文件变更触发整个 crate 重编译）
- 新开发者上手困难

**建议**: 按关注点拆分：
- `app_lifecycle.rs` — winit ApplicationHandler 实现
- `app_init.rs` — GPU 初始化、窗口创建
- `app_state.rs` — App struct 定义和公共方法

---

### 1.3 🟡 `UiShell` 每帧重建 Dock 子节点

**位置**: [ui_shell.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/ui_shell.rs) `update_frame()` 方法

```
self.dock.children.clear();  // 每帧清空
// ... 重新创建 Box<dyn Widget> ...
```

**问题**:
- 每帧分配 6-7 个 `Box<dyn Widget>`（TabBar, SearchBar, StatusBar, Sidebar, Scrollbar, EditorHost）
- 频繁的堆分配/释放
- Widget 内部状态每帧丢失（需要在外部保持状态）

**建议**: 改为**保持 Widget 实例**，仅更新其输入数据：
```rust
// 初始化时创建一次
self.dock.children = vec![...];

// 每帧只更新输入
self.tab_bar_widget.set_input(tab_input);
self.status_bar_widget.set_input(status_input);
```

---

### 1.4 🟡 三处重复的 shaping/顶点生成逻辑

**位置**: 
- [app_renderer.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/app_renderer.rs) — `render_text_fragments()`
- [render_pipeline.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/render_pipeline.rs) — `shape_visible_lines()`, `preedit_text_vertices()`
- [paint_backend.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/paint_backend.rs) — `emit_text()`

**问题**: atlas 查找 + 光栅化 + 顶点生成的模式在三处近乎相同。修改一处容易漏改另外两处。

**建议**: 提取统一的 `text_rasterizer` 模块：
```rust
pub fn rasterize_run(
    text: &str,
    x: f32, y: f32,
    font_size: f32, color: [f32; 4],
    shaper: &Shaper,
    atlas: &mut GlyphAtlas,
    queue: &Queue,
    vertices: &mut Vec<GlyphVertex>,
);
```

---

### 1.5 🟡 `DocumentView` 是上帝对象（80+ 字段）

**位置**: [document_view/](file:///Users/dan/proj/llmws/edit+/crates/app/src/document_view)

**问题**: `DocumentView` 包含 buffer、viewport、display map、render cache、cursor state、search state、language、highlighter 等所有文档相关状态，大部分字段 `pub(crate)`。

**影响**:
- 任何模块都能读写任何字段，缺乏封装
- 难以推理状态一致性
- 阻碍并行化（整个 struct 需要独占访问）

**建议**: 逐步拆分为子系统：
- `DocumentBuffer` — 文本 + undo
- `DocumentDisplay` — viewport + display map + render cache
- `DocumentCursor` — 光标 + 选区
- `DocumentSearch` — 搜索状态

---

### 1.6 🟢 `RenderContext` 定义位置不当

**现状**: `RenderContext` 定义在 [gutter.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/gutter.rs)，但被 `render_pipeline.rs` 和 `app_renderer.rs` 等多处使用。

**已知**: `lib.rs` 已经 re-export 了 `RenderContext`，使用端通过 `ui::RenderContext` 访问。

**建议**: 虽然 re-export 缓解了直接依赖，但语义上 `RenderContext` 不属于 gutter。建议移到 `ui::core` 或独立的 `ui::context` 模块。

---

## 二、Bug 清单

### 2.1 🔴 Stage 11（ICU 正则替换）被回退但残留影响

**最新提交** `345b5f3` 回退了 Stage 11：
- `icu::Regex` 和 `icu::Text` 被 stub 化（返回 `Err(Error(0))`）
- `EditCommand` 中移除了 `ToggleRegex`、`ToggleCase`、`ReplaceOne`、`ReplaceAll`
- 搜索栏高度从 42.0 降回 28.0

**潜在问题**:
- `SearchState` 简化后，如果有代码路径仍然访问被移除的字段，可能编译通过但行为异常
- ICU stub 返回 `Err` 但调用方可能没有正确处理错误
- 已有 ICU 相关测试被标记 `#[ignore]`，说明功能暂时搁置但代码未完全清理

**建议**: 做一次完整的 `grep` 确认没有死代码路径引用已 stub 的 ICU 功能。

---

### 2.2 🟡 `popup_menu.rs` NDC 与 px API 并存

**位置**: [popup_menu.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/popup_menu.rs) (31.8K)

**问题**: 旧的 NDC-based API（`context()`, `hit_test()`, `hovered_item()`, `popup_menu_vertices()`）和新的 px-based API 同时存在。注释提到"Phase 8 末尾删除"，但从 Phase 注释看 Phase 8 已完成。

**风险**: 调用方可能混用两套 API，导致坐标系不匹配（NDC vs px），表现为菜单位置偏移或点击穿透。

**建议**: 立即标记旧 API 为 `#[deprecated]`，在下一个迭代中删除。

---

### 2.3 🟡 `tab_infos` 在 `app_renderer.rs` 中重复构造

**位置**: [app_renderer.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/app_renderer.rs) L348 和 L400

**问题**: 同一帧内构造了两次 `tab_infos`（一次给 UiShell，一次给旧 tab bar），浪费计算且可能导致不一致。

**建议**: 提取为帧级缓存，构造一次后共享引用。

---

### 2.4 🟡 `content_hash` 计算逻辑重复

**位置**: 出现在 `render_pipeline.rs`、`app.rs::init_display_map()`、`reshape_worker.rs` 三处

**问题**: 使用 `wrapping_mul(31)` 的哈希链在三处独立实现。如果修改哈希算法（如换成更好的哈希函数），需要同步修改三处。

**建议**: 提取为 `fn content_hash(line: &str) -> u64` 函数。

---

## 三、安全风险

### 3.1 🔴 `Settings` 使用 `unsafe transmute` 延长 RefCell 生命周期

**位置**: [settings.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/settings.rs) L142-L158

```rust
pub fn get() -> std::cell::Ref<'static, Self> {
    SETTINGS.with(|s| {
        // SAFETY: SETTINGS is a thread_local static, so the borrow
        // lives for the program duration.
        let r: std::cell::Ref<'_, Settings> = s.borrow();
        unsafe { std::mem::transmute(r) }
    })
}

pub fn get_mut() -> std::cell::RefMut<'static, Self> {
    SETTINGS.with(|s| {
        let r: std::cell::RefMut<'_, Settings> = s.borrow_mut();
        unsafe { std::mem::transmute(r) }
    })
}
```

> [!CAUTION]
> **这是技术上不健全 (unsound) 的代码。**

**问题分析**:
1. `thread_local!` 的 `with()` 闭包中的引用生命周期绑定到闭包作用域，而非 `'static`
2. `transmute` 后，`Ref<'static>` 可以在 `with()` 闭包外继续存在
3. 如果同时持有 `get()` 返回的 `Ref` 和调用 `get_mut()` 的 `RefMut`，`RefCell` 的运行时借用检查将 panic
4. 更危险的场景：如果 `Ref<'static>` 跨越了 `thread_local` 的 destructor 执行点，可能导致 use-after-free

**实际风险**: 在当前单线程 UI 代码中，风险相对可控，但**任何未来的多线程重构都会暴露此问题**。

**修复方案**:

方案 A（最小改动）：在 `with()` 内完成所有操作，返回值而非引用：
```rust
pub fn font_size() -> f32 {
    SETTINGS.with(|s| s.borrow().font_size)
}
```

方案 B：使用 `OnceLock<RwLock<Settings>>`（但需注意死锁风险）

方案 C：使用 `arc-swap` crate 实现无锁读取

---

### 3.2 🟡 macOS FFI unsafe 块 — 可控但需持续关注

**位置**: [native_menu.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/native_menu.rs) (7 处), [sys/macos_titlebar.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/sys/macos_titlebar.rs) (1 处)

**评估**: 所有 unsafe 都是 objc2 FFI 调用，这是 macOS 平台必需的。代码使用了正确的 `#[unsafe(super(NSObject))]` 和 `#[unsafe(method(...))]` 标注。

**建议**: 
- 保持 unsafe 集中在 `native_menu.rs` 和 `sys/` 模块中
- 确保 `MenuTarget` 的 `retain` 和 `release` 对称调用
- 添加 `# Safety` 文档注释说明 invariants

---

### 3.3 🟡 `.unwrap()` 风险点

**生产代码中的高危 unwrap**:

| 位置 | 代码 | 风险 |
|------|------|------|
| [app.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/app.rs):1273 | `self.gpu.as_ref().unwrap()` | GPU 未初始化 → panic |
| [app.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/app.rs):1275 | `.clone().expect("FontSystem not initialized")` | 启动时序错误 → panic |
| [cursor_motion.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/cursor_motion.rs):190 | `ctx.advance_cache.last().unwrap()` | 空缓存 → panic |

> [!NOTE]
> 大部分 unwrap 出现在测试代码中（~50+ 处），这是可接受的。生产代码的 unwrap 数量相对可控，主要在初始化路径。

---

## 四、性能风险

### 4.1 🔴 `Theme` 每帧克隆 4 次

**分布**:
- [app_renderer.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/app_renderer.rs) L387, L602 — 渲染路径 2 次
- [events.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/events.rs) L53, L114 — **鼠标移动事件** 2 次

**问题**: `Theme` 包含 30+ 个 `[f32; 4]` 字段 + sidebar/search/menu 子结构。虽然都是 Copy 类型，**在鼠标移动事件路径上克隆尤其浪费**——鼠标移动事件触发频率极高（>100Hz）。

**建议**: 
- 立即改为 `&Theme` 引用传递
- 或使用 `Rc<Theme>`/`Arc<Theme>` 共享所有权

---

### 4.2 🔴 每帧重建 Widget 实例（堆分配）

**位置**: [ui_shell.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/ui_shell.rs) `update_frame()`

**问题**: 每帧：
1. `self.dock.children.clear()` — drop 所有 Widget Box
2. 重新创建 6-7 个 `Box<dyn Widget>`
3. 重新执行 Dock 布局

**影响**: 每帧 6-7 次堆分配 + 6-7 次堆释放 = 12-14 次 allocator 调用。在 60fps 下每秒 ~840 次不必要的内存管理操作。

**建议**: Widget 实例应该被持久化，仅通过 `set_input()` 更新数据。

---

### 4.3 🟡 `DisplayLineMap::set_entries()` 双重克隆

**位置**: [display_line_map.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/display_line_map.rs) L65-L136

**问题**: `set_entries()` 对 `Vec<DisplayLineEntry>` 先 clone 一份用于 SnapTree 构建，再保留原始 Vec。对大文件（万行级别）这意味着两倍内存分配。

**建议**: 使用 `take ownership` 模式，接受 `Vec` 所有权而非引用，然后只克隆给 SnapTree：
```rust
fn set_entries(&mut self, entries: Vec<DisplayLineEntry>) {
    self.tree = SnapTree::from(&entries);
    self.entries = entries; // moved, not cloned
}
```

---

### 4.4 🟡 `advance_cache.clone()` 可能很大

**位置**: [app.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/app.rs):2151

**问题**: `AdvanceCacheEntry` 的 Vec 大小与可见行中最长行的字符数成正比。对于非常长的行（>10K 字符），这个克隆代价不小。

**建议**: 如果只是传递给只读消费者，使用 `&[AdvanceCacheEntry]` 切片。

---

### 4.5 🟡 Sidebar 配置每帧克隆

**位置**: [ui_shell.rs](file:///Users/dan/proj/llmws/edit+/crates/app/src/ui_shell.rs) L328-L337

```
sidebar_config.clone()
sidebar_tabs.clone()
sidebar_persistent.clone()
```

**问题**: 每帧 3 次结构克隆用于 Widget 重建（与 4.2 相关）。

**建议**: 如果 Widget 持久化后，这些克隆自然消除。

---

### 4.6 🟢 `content_hash` 重复计算

**问题**: 同一行的哈希在 `render_pipeline.rs`、`init_display_map()`、`reshape_worker.rs` 中可能被重复计算。

**建议**: 将哈希结果缓存到 `DisplayLineEntry` 中，避免重复计算。

---

## 五、代码质量与可维护性

### 5.1 GPU 依赖渗透到 UI 层

**位置**: [gutter.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/gutter.rs), [decorations.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/decorations.rs)

**问题**: `gutter.rs` 直接使用 `wgpu::Texture`、`wgpu::Queue`、`render::GlyphAtlas`。`decorations.rs` 和 `render_geom.rs` 直接生成 `render::GlyphVertex`。

这违反了 AGENTS.md 中 UI 层的定位——UI 层应该输出抽象的 `DrawCmd`，由 app 层/backend 负责 GPU 细节。

**建议**: 这是从旧架构延续的问题。新的 `DrawCmd` 体系（`DrawCmd::FillRect`, `DrawCmd::Text`）已经定义了正确的抽象层级。逐步将 gutter/decorations 迁移到 DrawCmd 模式。

---

### 5.2 `Widget` trait 的 `as_any_mut()` 默认 `unimplemented!()`

**位置**: [widget.rs](file:///Users/dan/proj/llmws/edit+/crates/ui/src/core/widget.rs)

```rust
fn as_any_mut(&mut self) -> &mut dyn Any { unimplemented!() }
```

**问题**: 如果任何调用方对一个没有 override 此方法的 Widget 调用 `as_any_mut()`，程序立即 panic。

**建议**: 改为返回 `Option<&mut dyn Any>`，或提供安全的默认实现。

---

## 六、优先级排序

### P0 — 立即修复

| # | 问题 | 类别 | 工作量 |
|---|------|------|--------|
| 3.1 | Settings `transmute` unsound | 安全 | 中 |
| 4.1 | Theme 每帧克隆 4 次（含鼠标事件路径） | 性能 | 小 |
| 4.2 | Widget 每帧重建 | 性能 | 中 |

### P1 — 本迭代内修复

| # | 问题 | 类别 | 工作量 |
|---|------|------|--------|
| 1.1 | 新旧渲染路径并存（~130K 重复代码） | 架构/债务 | 大 |
| 2.1 | Stage 11 回退残留 | Bug | 小 |
| 2.2 | PopupMenu NDC/px API 并存 | Bug | 小 |
| 1.4 | 三处重复 shaping 逻辑 | 架构 | 中 |

### P2 — 下个迭代

| # | 问题 | 类别 | 工作量 |
|---|------|------|--------|
| 1.2 | app.rs 151KB 巨型文件 | 架构 | 中 |
| 1.5 | DocumentView 上帝对象 | 架构 | 大 |
| 4.3 | DisplayLineMap 双重克隆 | 性能 | 小 |
| 5.1 | GPU 依赖渗透 UI 层 | 架构 | 大 |
| 2.3 | tab_infos 重复构造 | Bug | 小 |
| 2.4 | content_hash 重复实现 | Bug | 小 |

### P3 — 长期改善

| # | 问题 | 类别 | 工作量 |
|---|------|------|--------|
| 1.3 | UiShell 每帧重建 Dock | 架构 | 中 |
| 1.6 | RenderContext 模块归属 | 架构 | 小 |
| 3.3 | 生产代码 unwrap | 安全 | 中 |
| 5.2 | as_any_mut unimplemented | 安全 | 小 |

---

## 七、总结

### 做得好的地方 ✅

1. **Widget 体系设计合理** — `Widget` trait + `Dock` 布局 + `DrawCmd` 抽象层是正确的方向
2. **依赖方向正确** — `ui` 不依赖 `app`，符合 AGENTS.md 分层规范
3. **Phase 渐进式重构** — 从 Phase 2 到 Phase 9 逐步推进，每个阶段有明确目标
4. **UI unsafe 集中** — ui crate 仅 settings.rs 有 2 处 unsafe，app 层的 unsafe 集中在 macOS FFI
5. **测试覆盖** — tab_bar 有 13K 的测试文件，render_pipeline 有 30K 测试

### 核心问题 ⚠️

1. **新旧代码并存是最大债务** — ~130K 的旧代码等待清理，增加维护负担和混淆风险
2. **Settings transmute 是安全隐患** — 虽然当前可工作，但阻碍任何并发改进
3. **每帧不必要的克隆和分配** — Theme 克隆、Widget 重建、DisplayLineMap 双重克隆，在大文件/高频交互场景下会成为瓶颈
4. **app.rs 151KB 是开发体验瓶颈** — 影响代码审查、导航、编译速度
