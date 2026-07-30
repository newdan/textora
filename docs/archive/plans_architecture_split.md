# 架构优化与大文件拆分详细计划

针对目前代码库中行数过多的 `app.rs` 和 `text_buffer.rs`，制定以下彻底拆分方案。
历史拆分方案中依然遗留了大量核心逻辑在主文件中，本次计划将强制下沉业务逻辑，让主对象（`App` 和 `TextBuffer`）真正成为轻量级的调度外观（Facade）。

## 1. `crates/core/src/buffer/text_buffer.rs` (现状: ~2100行)
**问题诊断**：目前该文件仍然包含了大量终端渲染 (`terminal-render`)、搜索替换算法 (`find_and_replace`) 以及光标计算 (`cursor_move_to_*`) 的代码。

### 阶段 1.1：剥离终端渲染模块 (`terminal_render.rs`)
* **目标**：将 `#[cfg(feature = "terminal-render")]` 相关的代码（约800+行）彻底移出。
* **内容**：
  * 将 `render()` 及辅助方法 `render_apply_highlights` 剥离。
  * 移动相关的终端视觉常量（`VISUAL_SPACE`, `VISUAL_TAB`）。
* **实施方式**：在 `crates/core/src/buffer/` 下新建 `terminal_render.rs`，通过 `impl TextBuffer` 或提供自由函数的方式扩展 `TextBuffer` 的渲染能力。

### 阶段 1.2：完善搜索模块剥离 (`search.rs`)
* **目标**：目前的 `search.rs` 仅有少量结构定义，核心的 `find_and_replace`, `find_and_replace_all`, `find_select_next` 都在 `text_buffer.rs` 中。
* **内容**：将 `find_and_select`, `find_and_replace`, `find_construct_search`, `find_parse_replacement` 全部移动到 `search.rs`，作为 `TextBuffer` 的扩展方法或委托给独立的结构。

### 阶段 1.3：下沉光标与导航逻辑 (`navigation.rs`)
* **目标**：将所有 `cursor_move_to_offset_internal`, `cursor_move_delta_internal`, `goto_line_start` 等处理全部剥离。
* **内容**：`navigation.rs` 应承载所有的光标位置计算和坐标转换（Logical / Visual / Offset）。主 `TextBuffer` 仅保留公共接口。

### 阶段 1.4：统一状态管理与清理
* **目标**：拆分完成后，`text_buffer.rs` 应该仅保留结构体定义、构造函数 (`new`)、部分基础 getter/setter，以及与 GapBuffer 直接交互的基础 API。整体行数控制在 500 行以内。

---

## 2. `crates/app/src/app.rs` (现状: ~3500行)
**问题诊断**：尽管之前抽取了部分逻辑，但 `App` 结构体仍然是一个上帝对象，包揽了文件/工作区管理（Tab 状态）、菜单交互处理以及生命周期和系统事件派发。

### 阶段 2.1：剥离菜单交互处理 (`menu_handler.rs`)
* **目标**：移除 `app.rs` 中繁杂的菜单方法 `dispatch_menu_action` 和 `execute_context_menu_action`。
* **内容**：
  * 创建 `crates/app/src/menu_handler.rs`。
  * 将相关的 Action 匹配转移至单独函数，传入必要的 App 子状态引用进行处理。
  * 移动 `context_menu_text_vertices` 生成逻辑。

### 阶段 2.2：剥离工作区与标签页管理 (`workspace.rs`)
* **目标**：将 `doc_views`, `active_index`, `pinned_indices`, `tab_history`, `preview_tab_index` 封装为独立的 `Workspace`。
* **内容**：
  * 将 `open_file`, `new_empty_tab`, `close_tab`, `switch_to`, `go_back`, `go_forward` 移动到 `Workspace` 的实现中。
  * `App` 只保留一个 `workspace: Workspace` 字段，大幅减少 `App` 主结构的字段数量，解除强耦合。

### 阶段 2.3：持久化与系统交互剥离 (`persistence.rs`)
* **目标**：移除 `load_pinned`, `save_pinned_paths`, `pinned_file` 等基于文件系统 I/O 的状态保存逻辑。
* **内容**：新建专门的持久化层，在 `App` 状态变更时触发。

### 阶段 2.4：主循环事件薄壳化
* **目标**：确保 `winit` 的 `ApplicationHandler` 或 `window_event` 内部回调保持极为精简的薄壳。
* **内容**：如果目前仍存在键盘、鼠标事件的巨大 match 代码块，将其包装为独立的方法提取至对应的 `input.rs` / `mouse.rs` 处理模块，主循环仅做中转和 `needs_redraw` 标记。

---

## 3. 跨文件生命周期解法策略 (解决“上帝对象”借用痛点)

在拆分巨型结构体（如 `App` 和 `TextBuffer`）时，为彻底规避跨文件间的 `&mut self` 生命周期冲突，本重构计划强制约束以下三种**零成本的优雅解法**：

### 策略 1：数据分组与视图模式 (Disjoint Structs)
**场景**：避免多个子系统抢占整个 `App` 的可变借用。
**解法**：不让状态平铺在主结构体中。将关联数据紧密打包（例如把 `doc_views`, `active_index` 等打包为 `Workspace`，`mouse_pos` 等打包为 `MouseState`）。在向子系统传参时，使用形如 `mouse::handle_input(&mut self.mouse, &mut self.workspace)` 的方式，利用 Rust 借用检查器对**不相交借用（Disjoint Borrows）**的原生支持，完美消除生命周期冲突。

### 策略 2：传递明确的依赖上下文 (Context Structs)
**场景**：子系统函数参数列表过长，导致代码难读（如渲染时需要传入5-6个散装引用）。
**解法**：在调用端现场组装轻量级的 `Context` 结构体：
```rust
pub struct RenderContext<'a> {
    pub text: &'a mut TextState,
    pub layout: &'a LayoutMetrics,
    pub settings: &'a Settings,
}
```
主循环构造 `let ctx = RenderContext { ... };` 传入，在保持函数签名整洁的同时，严格收敛了引用的生命周期。

### 策略 3：事件/命令驱动模式 (Command / Event Queue)
**场景**：如菜单处理（`menu_handler`），此类操作往往会网状式地修改各种不同的子状态，传引用非常极易造成借用冲突死锁。
**解法**：将其转化为**纯函数映射**。`menu_handler` 不再传入 `&mut App`，而是仅解析操作意图并返回 `AppCommand` 枚举（如 `AppCommand::CloseCurrentTab`）。主循环 `App` 收到指令后再在自己不涉及交叉借用的上下文中进行状态变动，从而物理切断了借用链。

---

## 执行建议与验收标准
1. **原子化提交**：严格按照每个子阶段单独进行重构。遇到跨文件引用的生命周期问题，通过调整为 `pub(crate)` 或传递确切引用解决。
2. **零回归**：每完成一个子阶段的重构，必须保证 `cargo check` 和 `cargo test` 能够完全通过。
3. **目标行数**：
   - `text_buffer.rs` 期望控制在 500 - 800 行。
   - `app.rs` 期望控制在 600 - 800 行。

请确认以上计划内容。确认后我们将按照阶段 1.1 开始逐步编写代码并重构。
