# 深度优化执行计划 (Deep Optimization Execution Plan)

根据 `plans_deep_optimization.md` 的目标，为确保代码改动可控、每次改动独立且可编译，我们将整个深度优化拆分为以下四个独立阶段进行。

## User Review Required

> [!IMPORTANT]
> 1. **关于 Action 机制的确认**：第一阶段将引入 Redux 风格的 `Action` 派发机制，`events.rs` 将不再直接修改 `&mut App`，而是返回 `AppAction` 枚举列表，由 `App::dispatch` 统一执行。**请确认此机制是否符合您的预期**。
> 2. **关于阶段切分的确认**：本次重构涉及大量核心文件，根据开发规范，将切分为四个阶段。每个阶段完成后都会进行验证并提交。**请确认以下切分方案**。

## 实施阶段切分

### 阶段一：Action 机制基础搭建与事件重构 (Stage 1)
**目标**：引入 `AppAction`，重构 `events.rs` 和 `app.rs` 的核心事件流。
- **文件变更**：
  - `[NEW] src/actions.rs`（或在现有文件定义 `AppAction`）
  - `[MODIFY] src/events.rs`：修改事件处理函数，返回 `Vec<AppAction>`。
  - `[MODIFY] src/app.rs`：实现 `App::dispatch(&mut self, action: AppAction)` 方法，调整顶层 Winit 事件调用。
- **验证**：确保基础事件（如调整窗口大小、简单按键）能通过 Action 正常流转并编译通过。

### 阶段二：Workspace 边界划分 (Stage 2)
**目标**：将 Tab 生命周期管理从 `App` 下沉至 `Workspace`，并引入 `WorkspaceEffect`。
- **文件变更**：
  - `[MODIFY] src/workspace.rs`：完善核心方法，返回 `WorkspaceEffect`。
  - `[MODIFY] src/app.rs` & `src/events.rs`：剥离原本在 App 中的 Tab 管理逻辑，改为调用 Workspace API 并处理 Effect。
- **验证**：Tab 的新建、切换、关闭（包括未保存提示）能正常工作。

### 阶段三：渲染管道抽象化 (`RenderContext`) (Stage 3)
**目标**：消除渲染子模块的冗长参数列表，解耦对 `App` 的直接依赖。
- **文件变更**：
  - `[MODIFY] src/render_pipeline.rs`：定义 `RenderContext`。
  - `[MODIFY] src/layout.rs`, `src/gutter.rs`, `src/decorations.rs`：修改接口以接收 `&mut RenderContext`。
- **验证**：文本渲染、语法高亮、行号、光标依然能正确绘制且无闪烁。

### 阶段四：App Core 进一步瘦身 (Stage 4)
**目标**：剥离 `App` 中残余的光标/视口状态和游离方法。
- **文件变更**：
  - `[MODIFY] src/app.rs`：提取 `cursor_pixel_x` 等状态。
  - `[MODIFY] src/cursor_motion.rs` & `src/viewport.rs`：接收相关逻辑并实现状态封装。
- **验证**：光标移动、跨行、PageUp/Down 及滚动功能计算无误。

## Verification Plan

在每个阶段完成后：
1. 运行 `cargo check --workspace` 确保没有生命周期或所有权冲突。
2. 运行 `cargo test` 确保现有测试覆盖（若有需要，将补充对应边界用例的测试）。
3. 进行手动功能检查以确保该阶段对应的核心交互未被破坏。
