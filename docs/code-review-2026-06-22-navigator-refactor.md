# Code Review: Navigator 重构 + 插件 Trait 完善 (2026-06-22)

**审查范围**: 11 个提交，从 `5b5dd147` 到 `60460e53`
**编译状态**: 通过 (`cargo check` 0 errors)
**测试状态**: 821 passed / 0 failed / 2 ignored

---

## 1. 总体概述

此轮提交完成三件事：

1. **引入 `Navigator` trait** — 将标签栏的滚动、渲染、命中测试抽象为 trait，`TabBarNavigator` 作为首个实现，替换原先散落在 `Workspace` 和 `App` 中的裸露字段。
2. **扩展 `ContentPlugin` trait** — 新增 `allows_editing()`、`toc_visible()`、`set_toc_visible()` 三个方法，消除所有通过 `downcast_ref::<MarkdownPlugin>()` 的运行时类型检查。
3. **性能测量基础** — 添加 debug-only 的帧计时埋点，验证 trait 动态分派无性能退化。

## 2. 架构设计评价

### 2.1 Navigator trait — ✅ 设计合理

```rust
pub trait Navigator: Any {
    fn render(&mut self, rect: Rect, ctx: &NavContext) -> NavOutput;
    fn hit_test(&self, pos_x: f32, pos_y: f32) -> Option<NavAction>;
    fn scroll(&mut self, delta: f32);
    fn tick(&mut self) -> bool;
    fn is_animating(&self) -> bool;
    fn scroll_offset(&self) -> f32;
    fn thickness(&self, dpi: f32) -> f32;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
```

**亮点**：
- `NavContext` 是纯数据 struct，APP 层从 `Workspace` 提取数据后传入，严格遵守跨层解耦红线。
- `NavEntry` 作为纯数据输入，不包含对 `DocumentView` 的引用，UI 层完全解耦。
- `tick()` 和 `is_animating()` 提供默认空实现，非滚动型 Navigator 无需关心。

**跨层解耦验证**：
```
Workspace/App → 提取 NavEntry[] + NavContext → Navigator::render()
                                                    ↓
                                              NavOutput (draw_list + actions)
```
UI 不依赖 `Workspace`，不依赖 `DocumentView` — **红线未破**。

### 2.2 ContentPlugin trait 扩展 — ✅ 消除 downcast

| 旧写法 | 新写法 | 影响 |
|--------|--------|------|
| `t.is_markdown()` → `downcast_ref::<MarkdownPlugin>()` | `!t.plugin.allows_editing()` | 6 处调用点 |
| `p.toc_visible` (直接字段访问 via downcast) | `self.plugin.toc_visible()` | `Tab::toc_visible()` |
| `md.toc_visible = visible` (downcast_mut) | `self.plugin.set_toc_visible(visible)` | `Tab::set_toc_visible()` |

彻底消除了 `Tab::is_markdown()` 方法以及 8 处 `downcast_ref`/`downcast_mut` 调用。符合 Phase 2 文档中规划的演进方向。

### 2.3 职责迁移 — ✅ Workspace 瘦身

从 `Workspace` 移除的字段和方法：
- `tab_scroll_offset: f32` → `TabBarNavigator.scroll_offset`
- `tab_scroll_target: f32` → `TabBarNavigator.scroll_target`
- `start_scroll_animation()` → `Navigator::scroll()`
- `tick_scroll_animation()` → `Navigator::tick()`
- `start_autoscroll()` → 合并入 `TabBarNavigator::render()`

Workspace 原先承担了太多 UI 职责，现在回归"文档集合管理"的核心语义。

## 3. 具体问题

### 3.1 ⚠️ 每次渲染都触发 autoscroll（行为变化）

`navigators/tab_bar.rs:568-572`:
```rust
// Auto-scroll to keep active tab visible (animated).
if let Some((target, _max)) =
    self.widget.autoscroll_target(ctx.active_index, self.scroll_offset)
{
    self.scroll_target = target;
}
```

旧代码中 autoscroll 只在 `update_tab_layout(true)` 被调用时触发（标签页切换、布局变更等事件驱动）。新代码改为**每帧渲染时**都执行 `autoscroll_target`。

**风险**：若用户手动滚动画看到某个标签后，active tab 恰好落在视口外，autoscroll 会在下一帧立刻覆盖用户的 `scroll_target`，导致无法手动滚动远离 active tab。这是一个 UX 行为变更，需要确认是有意为之。

**建议**：如果这是有意的（始终保证 active tab 可见），注释应明确说明此行为。如果是疏忽，autoscroll 应保留事件驱动语义，仅在 `NavAction::SwitchTo` 等离散事件时触发。

### 3.2 ⚠️ 每帧打开 `/tmp/perf.log` 文件（debug build 性能影响）

`app_lifecycle.rs:450-458`:
```rust
#[cfg(debug_assertions)]
{
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/perf.log")
        ...
}
```

每个 `RedrawRequested` 和 `AboutToWait` 事件都执行一次文件打开→写入→关闭。60fps 下每秒 60 次文件操作，会显著拖慢 debug build。

**建议**：缓存 `BufWriter` 或使用 `lazy_static!` 持有句柄。或者在测量期结束后移除这段代码，避免留在代码库中（当前注释标注为 "debug only"，但长期存在会增加维护负担，也违反 CLAUDE.md 中关于删除死代码的规定）。

### 3.3 💡 帧计时输出从 stdout 改到 stderr

`app_renderer.rs:834`:
```rust
// 旧: println!("[frame] total=...");
// 新: eprintln!("[frame] total=...");
```

`println!` 改为 `eprintln!` 是合理的选择（性能日志不应污染正常 stdout 输出），但需要注意：`eprintln!` 可能被终端缓冲策略影响。如果需实时观察，建议显式 `flush`。

### 3.4 💡 `NavOutput.actions` 始终为空

`navigators/tab_bar.rs:574`:
```rust
let actions = Vec::new();
```

`NavOutput` 设计了 `actions: Vec<NavAction>` 字段，且 `NavAction` enum 定义了 8 种动作，但目前 `TabBarNavigator::render()` 始终返回空列表。实际 tab 操作事件仍走 `UiShell` 的 Widget 事件路径。

**建议**：如果 actions 是为未来 Sidebar Navigator 预留的，加一行注释说明。否则属于死代码，应清理。

### 3.5 💡 `hit_test()` 返回 None — 命中测试路径不统一

```rust
fn hit_test(&self, _pos_x: f32, _pos_y: f32) -> Option<NavAction> {
    // TabBar hit testing is handled via the Widget::on_event path in UiShell.
    None
}
```

Trait 定义了 `hit_test()` 接口但 TabBar 实现未用，命中测试仍走旧的 `Widget::on_event` 路径。这导致 Navigator trait 的抽象存在缺口：将来 Sidebar 会走 `hit_test()` 路径而 TabBar 不走，使用者必须知道这个差异。

**建议**：要么在 trait 文档中明确 `hit_test` 为可选的（提供默认实现），要么统一让 TabBar 也走这一路径。

## 4. Bug 修复验证

### 4.1 双倍动画 tick ✅ 已修复

旧代码：
- `app_renderer.rs`: `if self.workspace.tick_scroll_animation() { ... }` ← 1
- `dispatch/tabs.rs`: `let animating = self.workspace.tick_tab_scroll();` ← 2

每帧 tick 两次，动画速度翻倍。新代码仅在 `app_renderer.rs` 末尾调用 `self.workspace.navigator.tick()`，消除重复。

### 4.2 `update_tab_layout(autoscroll)` 参数消除 ✅

`autoscroll: bool` 参数仅控制是否触发自动滚动，但调用方全传 `true`（除了旧 `chrome.rs` 中一个已改为不调用 `update_tab_layout` 的路径）。简化后方法无参数，职责单一。

### 4.3 滚动步长常量化 ✅

`dispatch/chrome.rs:74`:
```rust
let step = viewport_width * 0.7;
```

相比旧代码直接在表达式处计算，现在是局部变量，语义清晰。

## 5. 测试覆盖

`workspace.rs` 中 `test_autoscroll_active_tab` 被重写为 `test_navigator_initialized_on_workspace`：

```rust
fn test_navigator_initialized_on_workspace() {
    // 验证 Navigator 存在、scroll_offset 初始为 0、thickness 正值
    // 不再测试滚动的具体行为（已移至 Navigator trait 层）
}
```

**评价**：旧测试直接操作 `tab_scroll_offset`/`tab_scroll_target` 等内部字段，新测试改为通过 `Navigator` trait 公共接口验证。这是好的方向，但 `TabBarNavigator` 本身缺少独立的单元测试（如 `tick_animation` 的收敛性、`scroll` 的 clamp 行为）。

**建议**：为 `TabBarNavigator` 添加单元测试：
- `scroll` 不应超出 `max_scroll`
- `tick_animation` 动画收敛（从 target=200, offset=0 开始，经 N 帧后 offset 应等于 target）

## 6. 性能测量埋点

`app.rs` 新增 `render_frame_count: u32`，在 `app_renderer.rs` 中利用 `wrapping_add(1)` 和 `is_multiple_of(60)` 实现每 60 帧一次的周期性摘要。

**评价**：`wrapping_add` 是帧计数器的正确选择（避免 panic on overflow），`is_multiple_of(60)` 无需分支预测。

设计文档中预期数值：

| 场景 | 预期 | 合理性 |
|------|------|--------|
| 空闲 `.rs` 文件 | < 2ms | 合理 — 无密集渲染 |
| 空闲 `.md` 预览 | < 5ms | 合理 — Markdown 渲染较重 |
| 快速 `.md` 滚动 | < 8ms | 略显乐观 — 滚动时 reshape 队列压力大 |
| 快速切标签 | < 3ms | 合理 |

结构设计的性能影响分析（trait vtable dispatch vs `match` enum）在文档中已有说明并符合预期：单次虚函数调用开销为纳秒级，可忽略。

## 7. 总结

| 维度 | 评级 | 说明 |
|------|------|------|
| 架构设计 | ✅ 优秀 | Navigator trait 解耦清晰，Cross-layer 红线保持 |
| 代码质量 | ✅ 良好 | 消除 downcast、消除重复 tick、消除死代码 |
| 测试 | ⚠️ 可改进 | TabBarNavigator 缺少独立单元测试 |
| 性能影响 | ⚠️ 注意 | Debug build 中每帧写文件的埋点需评估 |
| UX 行为 | ⚠️ 需确认 | Per-frame autoscroll 行为变化需明确意图 |

**合并建议**：整体质量良好，建议在确认 3.1（autoscroll 行为变化）和 3.2（perf 日志文件开销）后合并。
