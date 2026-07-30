# edit+ 工程结构审计报告

> 日期：2026-06-14
> 范围：全工程（根目录 + 7 个 crate + docs + scripts + tests + assets）

---

## 1. 总览

| 指标 | 数值 |
|------|------|
| crate 数量 | 7（app, core, ui, render, shaping, lsh, stdext） |
| Rust 源码总行数 | ~55,200 |
| 最大 crate | app（22,388 行） |
| 已跟踪文档 | 52 个 |
| 根目录散落文件 | 16 个（含已跟踪和未跟踪） |

---

## 2. 根目录垃圾文件

### 2.1 应删除的已跟踪文件

| 文件 | 问题 |
|------|------|
| `crash.log` | 旧崩溃日志，不应入库。`.gitignore` 已有 `*.log` 规则，说明此文件在规则添加前就已跟踪 |
| `CLAUDE.md` | 与 `AGENTS.md` 内容几乎一致（仅标题和计划路径不同），存在内容漂移。应删除 `CLAUDE.md`，只保留 `AGENTS.md` |

### 2.2 未跟踪但应清理的文件

这些文件被 `.gitignore` 排除，但仍占用磁盘空间，建议定期清理或移入 `scripts/`：

| 文件 | 用途 | 建议 |
|------|------|------|
| `fix_app_tests.py` | 一次性 sed 脚本（批量修改 assert 值） | 删除 |
| `fix_popup_menu.py` | 一次性代码修补脚本 | 删除 |
| `fix_ui_shell.py` | 一次性结构体修复脚本 | 删除 |
| `update_dock.rs` | 一次性 patch 脚本 | 删除 |
| `test_format.rs` | 两行的独立测试片段 | 删除 |
| `test_visible_rows.rs` | 独立计算验证片段 | 删除 |
| `.git_log_10.txt` | git log 输出，调试遗留 | 删除 |
| `plans.md` | 根目录实施方案（已在 `docs/` 有大量 plans） | 移入 `docs/` 或删除 |

---

## 3. 文档膨胀（docs/）

### 3.1 现状

- **52 个已跟踪文档**，占 1.8MB
- 大量是已完成阶段的审计/分析报告，不再有参考价值
- `docs/superpowers/` 下有 14 个 plans 和 4 个 specs

### 3.2 建议归档的过期文档

以下文档属于已完成阶段的审计和分析，建议移入 `docs/archive/` 或删除：

**已完成阶段审计（Stage 6/7/9 等）：**
- `stage6_audit.md`、`stage7_review.md`、`stage_6_7_audit.md`
- `progress_audit.md`、`audit_fix.md`、`audit_fix_v2.md`

**已解决 Bug 分析：**
- `ghost_lines_root_cause_v2.md`、`scroll_bugs_root_cause.md`
- `workspace_restore_bug_analysis.md`
- `cursor-click-drift-investigation.md`

**已实施的设计文档：**
- `viewport_0601.md`、`viewport-scroll-redesign.md`、`viewport_architecture_analysis.md`
- `displayrow.md`、`displayrow_review.md`
- `visual_doc_design.md`
- `plans_large_file_scroll_perf.md`（大部分内容已实现）

**重复/被取代的文档：**
- `ui-skeleton-audit-2025-06-12.md` vs `ui-skeleton-audit-2026-06-12.md`（两个日期版本共存）
- `audit_ui_skeleton_2025_06_12.md`（与上面重复）
- `plans_deep_optimization.md` + `plans_deep_optimization_execution.md`（可合并）

### 3.3 建议保留的文档

- `manual_test_protocol.md`（持续使用）
- `editor_performance_playbook.md`（参考价值）
- `plan-ui-split.md`（架构设计参考）
- `plans-sidebar-item-features.md`、`plans-splitter-widget.md`（当前活跃）
- `docs/superpowers/specs/` 下的设计文档

---

## 4. UI 模块双层架构（最严重的结构问题）

### 4.1 现状

UI crate 存在两套并行模块：

```
ui/src/
├── scrollbar.rs        (162 行) ← 旧：纯函数 compute_layout_px + ScrollbarLayoutPx
├── sidebar.rs          (1646 行) ← 旧：完整状态机 + SidebarState + SidebarAction
├── status_bar.rs       (100 行) ← 旧：StatusBarInput + build_text
├── popup_menu.rs       (~400 行) ← 旧：PopupMenu + PopupMenuAction + ContextMenuAction
├── title_bar.rs        (~150 行) ← 旧：TitleBarInput + 渲染函数
├── text_renderer.rs    (~80 行) ← TextFragment 定义
├── widgets/
│   ├── scrollbar.rs    ← 新：ScrollbarWidget 包装旧模块
│   ├── sidebar.rs      (1408 行) ← 新：SidebarWidget 包装旧模块
│   ├── status_bar.rs   ← 新：StatusBarWidget 包装旧模块
│   ├── popup_menu.rs   ← 新：PopupMenuWidget 包装旧模块
│   ├── title_bar.rs    ← 新：TitleBarWidget 包装旧模块
│   ├── search_bar.rs   ← 新：SearchBarWidget（无旧对应）
│   ├── tab_bar.rs      ← 新：TabBarWidget
│   ├── list.rs         ← 新：VerticalListWidget
│   └── ...
```

**问题：**
- 旧模块定义了数据类型、状态管理、action 枚举
- 新 widgets 只是薄包装，通过 `use crate::xxx` 引用旧模块
- app 层混合使用两套：`ui::sidebar::SidebarAction` + `ui::widgets::sidebar::SidebarWidget`
- `sidebar.rs` 一个组件就有 3054 行（1646 + 1408），严重超标

**建议：** 将旧模块的类型/状态/action 逐步迁移到 `widgets/` 下对应文件，旧模块最终只保留纯函数（如 `compute_layout_px`），或合并为一个 `types.rs`。

### 4.2 `ui::core` 命名冲突

`ui/src/core/` 是一个模块目录（定义 Widget trait、Rect、DrawCmd 等），但它与 Rust 标准库的 `core` crate 以及本项目的 `edit-plus-core` crate 名称冲突。

在 `ui/src/decorations.rs` 中：
```rust
use core::highlight::HighlightKind;  // 这是 edit-plus-core，不是 std::core
```

这会导致混淆。建议将 `ui/src/core/` 重命名为 `ui/src/framework/` 或 `ui/src/primitives/`。

---

## 5. 孤立文件

### 5.1 `app_tests.rs`（孤儿测试文件）

路径：`crates/app/src/app_tests.rs`（1159 行）

- 文件开头有 `use super::*`，说明它应该是某个模块的子模块
- 但 `git grep "mod app_tests"` 无结果——没有任何文件通过 `mod app_tests;` 引入它
- 内容是 `compute_visual_lines` 等函数的测试
- `render_pipeline_tests.rs` 已经覆盖了类似内容（通过 `#[path = "render_pipeline_tests.rs"] mod tests;`）

**建议：** 检查是否与 `render_pipeline_tests.rs` 重复。如果是，删除 `app_tests.rs`；如果测试不同，找合适父模块引入。

### 5.2 `render_pipeline_tests.rs` 的 `#[path]` 模式

`crates/app/src/render_pipeline.rs:946`：
```rust
#[path = "render_pipeline_tests.rs"]
mod tests;
```

这是合法的 Rust 但非标准做法。通常测试直接放在文件底部的 `#[cfg(test)] mod tests { ... }` 中。

**建议：** 保持现状（744 行测试代码单独放也有道理），但可以在文件头部加注释说明。

---

## 6. 死代码和未使用的模块

### 6.1 `terminal_stubs.rs` + `terminal_render.rs`

| 文件 | 行数 | 状态 |
|------|------|------|
| `core/src/terminal_stubs.rs` | ~90 行 | 定义 `Framebuffer`、`Clipboard`、`IndexedColor` 等桩类型 |
| `core/src/buffer/terminal_render.rs` | ~100 行 | 全部在 `#[cfg(feature = "terminal-render")]` 保护下 |

- `terminal-render` feature 从未被任何 crate 的 Cargo.toml 启用
- `terminal_stubs.rs` 仅被 `terminal_render.rs` 使用
- 这些是从 microsoft/edit 继承的终端渲染遗留代码

**建议：** 如果短期不做终端渲染模式，可以删除这两个文件及 `core/Cargo.toml` 中的 `terminal-render` feature。

### 6.2 `stdext` 中未使用的模块

| 模块 | app+ui+core 中的引用次数 |
|------|--------------------------|
| `stdext::alloc` | 0 |
| `stdext::glob` | 0 |
| `stdext::helpers` | 0（通过 `pub use helpers::*` 间接导出） |
| `stdext::simd` | 1 |

`alloc`、`glob` 从 microsoft/edit 整体 vendor 进来但未使用。

**建议：** 标记为 `#[cfg(feature = "vendored")]` 或直接移除未使用模块。

### 6.3 `core/src/cell.rs`

- 被 `text_buffer.rs` 和 `edit.rs` 使用，**不是死代码**
- 但有 `#[allow(unused)]` 标记在 `debug` 模块上
- 注释说明是因为 debug 模块的类型别名在 release 构建下不使用

**状态：** 正常，`#[allow(unused)]` 是合理的。

---

## 7. 依赖问题

### 7.1 `cosmic-text` 重复声明

```toml
# crates/app/Cargo.toml
cosmic-text = { version = "0.12", default-features = false, features = ["std", "swash"] }

# crates/shaping/Cargo.toml
cosmic-text = { version = "0.12", default-features = false, features = ["std", "swash"] }
```

app 通过 `shaping` 间接依赖 `cosmic-text`。app 直接使用它只是为了 `cosmic_text::fontdb::ID` 等类型。

**建议：** 将 `cosmic-text` 声明移到 `[workspace.dependencies]`，并在 `shaping` 中 re-export app 需要的类型，让 app 不直接依赖 `cosmic-text`。

### 7.2 `unicode_categories` 重复声明

app 和 ui 都直接依赖 `unicode_categories`，可以统一到 workspace.dependencies。

 
---

## 8. 文档/配置冗余

### 8.1 `CLAUDE.md` vs `AGENTS.md`

| 差异点 | AGENTS.md | CLAUDE.md |
|--------|-----------|-----------|
| 标题 | `# AGENTS.md` | `# CLAUDE.md` |
| 计划路径 | `./docs/plans*.md` | `plans*.md` |
| 项目架构 | 包含完整架构说明（依赖层次、UI 模块一览、设计决策） | 不包含 |

`AGENTS.md` 是超集。`CLAUDE.md` 应删除。

### 8.2 `plans.md` 在根目录

根目录的 `plans.md` 是项目的总实施方案，与 `docs/plans*.md` 命名规范冲突。

**建议：** 移入 `docs/plans-overview.md`。

---

## 9. 测试组织

### 9.1 `document_view/` 测试文件命名

```
test_b11_tests.rs        (50 行)
test_boundary_tests.rs   (521 行)
test_cursor_visual_tests.rs (484 行)
test_perf_tests.rs       (37 行)
test_stage7_tests.rs     (591 行)
test_tests.rs            (470 行)
test_word_wrap_tests.rs  (40 行)
```

- 命名不统一：`test_tests.rs` 意义不明，`test_b11_tests.rs` 中 `b11` 含义不清
- `test_` 前缀重复（文件名有 `test_`，内部又是 `#[test]`）
- 共 2193 行，拆分粒度合理但命名需整理

**建议：** 
- `test_tests.rs` → `basic_tests.rs` 或 `core_tests.rs`
- `test_b11_tests.rs` → 用有意义的名称（如 `resize_tests.rs`）
- 统一去掉 `test_` 前缀，改为 `xxx_tests.rs`

### 9.2 `tests/golden/` 目录

包含一个 1.4MB 的 `hello_edit_plus.ppm` golden 文件。被 `render_smoke.rs` 使用。

**建议：** 确认 golden test 在 CI 中运行。PPM 文件较大，考虑压缩或用 git-lfs。

---

## 10. 代码量过大的文件

| 文件 | 行数 | 建议 |
|------|------|------|
| `app/src/document_view/mod.rs` | ~1286 | 已拆出 cursor/display，剩余仍偏大 |
| `app/src/render_pipeline.rs` | 1033 | 核心渲染逻辑，复杂度合理 |
| `ui/src/sidebar.rs` | 1646 | 与 `widgets/sidebar.rs`(1408) 合并后更大 |
| `app/src/app.rs` | 很大 | 多个 `impl` 块，考虑按职责拆分 |
| `ui/src/popup_menu.rs` | ~400 | 文档注释重复 5 遍（bug） |

### 10.1 `popup_menu.rs` 文档注释重复

```rust
//! Popup menu — right-click context menu and overflow menu.
//! Popup menu — right-click context menu and overflow menu.
//! Popup menu — right-click context menu and overflow menu.
//! Popup menu — right-click context menu and overflow menu.
//! Popup menu — right-click context menu and overflow menu.
```

同样的 doc comment 重复了 5 次。

---

## 11. 架构层面的建议

### 11.1  `render` crate vs app 的渲染文件

| 位置 | 职责 |
|------|------|
| `crates/render/` (789 行) | GlyphAtlas + GlyphRenderer + wgpu pipeline |
| `app/src/render_pipeline.rs` (1033 行) | 可视行计算 + 批处理逻辑 |
| `app/src/render_cache.rs` (360 行) | 帧级缓存 |
| `app/src/render_state.rs` (129 行) | GPU 状态管理 |
| `app/src/paint_backend.rs` (554 行) | 最终绘制后端 |
| `app/src/text_rasterize.rs` (~100 行) | 字形光栅化 |
| `ui/src/text_renderer.rs` (~80 行) | TextFragment 定义 |

7 个文件分散在 3 个 crate 中处理渲染，职责边界模糊。

**建议：** 
- `render` crate 应包含所有 GPU 相关逻辑（atlas + renderer + pipeline + state）
- `app` 只保留业务层（可视行计算、缓存策略、批处理调度）
- `text_renderer.rs`（定义）和 `text_rasterize.rs`（实现）可合并

### 11.3 二进制名称不匹配

```toml
# crates/app/Cargo.toml
[[bin]]
name = "NoteR"
```

项目叫 `edit+`，二进制叫 `NoteR`。建议统一。

---

## 12. 优化优先级排序

| 优先级 | 项目 | 工作量 | 影响 |
|--------|------|--------|------|
| P0 | 删除 `crash.log` 的 git 跟踪 | 1 分钟 | 消除仓库噪声 |
| P0 | 删除 `CLAUDE.md` | 1 分钟 | 消除配置漂移 |
| P1 | 清理根目录散落文件 | 5 分钟 | 整洁工作区 |
| P1 | 修复 `popup_menu.rs` 重复注释 | 1 分钟 | 代码质量 |
| P1 | 孤儿 `app_tests.rs` 处理 | 15 分钟 | 消除编译警告 |
| P2 | 归档过期 docs（~20 个文件） | 30 分钟 | 减少认知负担 |
| P2 | UI 双层架构合并 | 2-3 天 | 架构清晰度 |
| P2 | `ui::core` 重命名 | 2 小时 | 消除命名混淆 |
| P3 | `cosmic-text` 统一到 workspace | 30 分钟 | 依赖管理 |
| P3 | 清理 `stdext` 未使用模块 | 1 小时 | 代码瘦身 |
| P3 | 删除 `terminal_stubs` + `terminal_render` | 30 分钟 | 消除死代码 |
| P3 | 渲染文件职责重新划分 | 1-2 天 | 架构清晰度 |

---

## 13. 总结

工程整体架构设计合理，crate 分层清晰。最突出的问题是：

1. **UI 双层架构过渡期**：旧模块和新 widgets 并存，类型定义分散
2. **文档过载**：52 个已跟踪文档，大部分是过期的审计/分析报告
3. **根目录杂乱**：散落的 Python 脚本、测试片段、crash log
4. **渲染文件职责分散**：7 个文件跨 3 个 crate

建议按 P0→P1→P2→P3 优先级逐步清理。

---

## 14. 重复代码与类型深度分析（补充）

### 14.1 `Rect` 定义重复（跨 crate）

| 位置 | 字段 | 类型 | 用途 |
|------|------|------|------|
| `core/src/helpers.rs` | `left, top, right, bottom` | `CoordType (isize)` | padding/inset 矩形（CSS 风格） |
| `ui/src/core/geom.rs` | `x, y, w, h` | `f32` | 屏幕矩形（像素坐标） |

两者都有 `contains()`、`is_empty()` 等方法，概念相同但实现不同。

**分析：** `core::helpers::Rect` 实际上是 `padding/inset` 概念（四个方向的值），不是屏幕矩形。名字叫 `Rect` 容易混淆。仅在 `core/benches/cursor_nav.rs` 中引用了 `Point`（不是 `Rect`），`Rect` 本身几乎未使用。

**建议：**
- `core::helpers::Rect` 重命名为 `Inset` 或 `EdgeInsets`（更准确反映语义）
- 或直接删除（如果 padding 逻辑可以用 4 个参数代替）

---

### 14.2 `mock_cluster` 测试辅助函数重复

| 文件 | 函数 |
|------|------|
| `app/src/render_pipeline_tests.rs` | `fn mock_cluster(byte_start, byte_end, advance) -> GlyphCluster` |
| `app/src/app_tests.rs` | `fn mock_cluster(byte_start, byte_end, advance) -> GlyphCluster` |

两个函数实现完全相同，都构造 `shaping::GlyphCluster`。

**建议：** 提取到 `app/src/test_helpers.rs` 或 `#[cfg(test)] pub mod test_utils`，两个测试文件共用。

---

### 14.3 旧模块 vs Widget 层：双重 hit_test

| 层 | PopupMenu | 逻辑 |
|----|-----------|------|
| 旧 `ui/src/popup_menu.rs:304` | `pub fn hit_test_px(&self, px, py) -> Option<&PopupMenuAction>` | 返回具体 action |
| 新 `ui/src/widgets/popup_menu.rs:62` | `fn hit(&self, px, py) -> bool` | 仅返回是否命中 |

旧模块有完整命中逻辑（返回哪个菜单项被点），新 Widget 只包装了 `bool`。这意味着：
- 如果需要知道具体命中了哪个 item，必须调用旧的 `hit_test_px`
- Widget 的 `hit()` 只用于 Dock 的事件分发判断

**同样的模式出现在 Sidebar：**
- `sidebar.rs` 有完整的事件处理 + 状态转换
- `widgets/sidebar.rs` 的 `on_event()` 只是转发

**这不是 bug，但说明 Widget 抽象层尚未完全封装旧逻辑。**

---

### 14.4 双重 SidebarState 绘制

| 文件 | 方法 | 行数 |
|------|------|------|
| `ui/src/sidebar.rs:1646` | `pub fn paint(&self, ctx, active_index)` | ~400 行绘制逻辑 |
| `ui/src/widgets/sidebar.rs:1408` | `fn paint(&self, ctx)` (Widget trait) | 调用 `self.state.paint(ctx, ...)` |

Widget 层的 `paint` 委托给旧模块的 `paint`，但 Widget 层也内嵌了 `VerticalListWidget` 来渲染 items。

**问题：** 绘制逻辑分散在两层——框架（bg/header/buttons）在旧模块，items 在新 Widget。

---

### 14.5 三套命令分发体系

| 文件 | 类型 | 职责 |
|------|------|------|
| `input.rs` | `EditCommand` enum | 文本编辑命令（cursor up/down, delete, etc.） |
| `commands.rs` | `execute_edit_command_v2()` | 执行 `EditCommand` 的分发函数 |
| `menu_handler.rs` | `AppCommand` enum | 应用级命令（Quit, Open, Save, CloseTab） |
| `native_menu.rs` | `MenuAction` enum | 原生菜单栏动作（对应菜单 tag） |

**分发链：** `MenuAction → AppCommand → 执行` 和 `键盘 → EditCommand → execute_edit_command_v2`

两套命令体系（编辑 vs 应用）是合理的分离。但 `AppCommand` 的 variants（`Quit`, `NewEmptyTab`, `OpenFileDialog`, `SaveActiveTab`, `CloseTab`, `CloseOthers`, `CloseRight`, `CloseAll`）与 `MenuAction` 高度重叠。

**建议：** 考虑让 `MenuAction` 直接映射到 `EditCommand` 或合并 `AppCommand` 和 `MenuAction`。

---

### 14.6 `HEADER_H` 硬编码重复

```rust
// ui/src/sidebar.rs
const HEADER_H: f32 = 28.0;

// ui/src/title_bar.rs (注释引用)
//! Height matches the sidebar header (HEADER_H = 28px * dpi).

// ui/src/widgets/title_bar.rs (注释引用)
//! 高度与 sidebar header 一致（HEADER_H = 28px * dpi）。

// ui/src/widgets/search_bar.rs
pub const SEARCH_BAR_HEIGHT: f32 = 28.0;  // 同样的 28.0
```

`28.0` 出现了 3 次，但只有 `sidebar.rs` 用 `const` 命名。`search_bar` 碰巧也是 28。

**建议：** 提取 `const UI_BAR_HEIGHT: f32 = 28.0;` 到 `ui/src/settings.rs` 或一个 `constants.rs`。

---

### 14.7 DPI scale 获取模式重复

`Settings::with(|s| s.dpi_scale)` 在 app 层出现了 **68 次**（`app.rs` 一个文件就有大量）。

```rust
let dpi = Settings::with(|s| s.dpi_scale);       // 反复出现
let dpi = ui::settings::Settings::with(|s| s.dpi_scale);  // 有时用全路径
```

**建议：** 在 `App` 结构体上加一个 `fn dpi(&self) -> f32` 方法，或在每帧开始时缓存到局部变量。

---

### 14.8 `core::helpers` 中的类型与 `std` 重复

| 类型 | 说明 | 是否必要 |
|------|------|----------|
| `Point { x: CoordType, y: CoordType }` | 与 `std` 无直接对应，但 `CoordType = isize` | 仅 bench 使用 |
| `Size { width, height }` | 有 `as_rect()` 方法 | 检查使用量 |
| `MetricFormatter<T>` | 格式化工具 | 检查使用量 |
| `file_read_uninit` | 文件 IO 工具 | 检查使用量 |

`Point` 和 `Size` 是 microsoft/edit 的遗留类型，如果仅在 bench 或极少数地方使用，可以考虑移除。

---

### 14.9 `tab_bar/mod.rs` 的 re-export 链

```rust
pub use crate::popup_menu::{       // re-export popup_menu 的类型
    ContextMenuAction, PopupMenu, PopupMenuAction, PopupMenuItem,
};
pub use types::{TabInfo, TabBarCtx, tab_bar_height};
pub use state::{TabBarInput, TabBarAction, TabBarState};
pub use crate::core::widget::MouseButton;
```

`tab_bar/mod.rs` 作为"门面" re-export 了 `popup_menu` 的全部类型。这意味着调用者可以 `use ui::tab_bar::PopupMenu` 而不用直接引用 `ui::popup_menu`。

**问题：** 这使得 tab_bar 模块承担了不应该属于它的 re-export 职责。

---

### 14.10 重复汇总表

| # | 重复项 | 位置 | 严重度 |
|---|--------|------|--------|
| 1 | `Rect` 两个定义（core vs ui） | `core/helpers.rs` + `ui/core/geom.rs` | ⚠️ 中 |
| 2 | `mock_cluster` 测试辅助 | `render_pipeline_tests.rs` + `app_tests.rs` | 低 |
| 3 | 旧模块 hit_test vs Widget hit | `popup_menu.rs` + `widgets/popup_menu.rs` | 低（设计如此） |
| 4 | 旧 Sidebar paint vs Widget paint | `sidebar.rs` + `widgets/sidebar.rs` | ⚠️ 中 |
| 5 | MenuAction ≈ AppCommand | `native_menu.rs` + `menu_handler.rs` | ⚠️ 中 |
| 6 | HEADER_H / 28.0 硬编码 | 3 处 | 低 |
| 7 | `Settings::with(|s| s.dpi_scale)` 68 次 | `app.rs` 为主 | ⚠️ 中 |
| 8 | `core::helpers` Point/Size/Rect | microsoft/edit 遗留 | 低 |
| 9 | tab_bar re-export popup_menu | `tab_bar/mod.rs` | 低 |
| 10 | `cosmic-text` 双重依赖 | app + shaping | ⚠️ 中 |
| 11 | UI 旧模块 vs widgets/ 双层 | 5 个组件 × 2 层 | 🔴 高 |

---

## 15. 硬编码值全量梳理

### 15.1 已定义为 `const` 的常量（良好的）

| 常量 | 值 | 文件 |
|------|-----|------|
| `HEADER_H` | 28.0 | `ui/src/sidebar.rs` |
| `ROW_H` | 24.0 | `ui/src/sidebar.rs` |
| `NEW_BTN_H` | 28.0 | `ui/src/sidebar.rs` |
| `SETTINGS_BTN_H` | 28.0 | `ui/src/sidebar.rs` |
| `PADDING` | 6.0 | `ui/src/sidebar.rs` |
| `EDGE_RESIZE_W` | 4.0 | `ui/src/sidebar.rs` |
| `SEARCH_BAR_HEIGHT` | 28.0 | `ui/src/widgets/search_bar.rs` |
| `SCROLLBAR_RESERVE_PX` | 14.0 | `ui/src/scrollbar.rs` |
| `SCROLLBAR_THUMB_W_IDLE` | 4.0 | `ui/src/scrollbar.rs` |
| `SCROLLBAR_THUMB_W_ACTIVE` | 14.0 | `ui/src/scrollbar.rs` |
| `PIN_BAR_WIDTH_LOGICAL` | 2.0 | `ui/src/widgets/list.rs` |
| `PIN_BAR_MARGIN_LOGICAL` | 6.0 | `ui/src/widgets/list.rs` |
| `CLOSE_BTN_SIZE_LOGICAL` | 12.0 | `ui/src/widgets/list.rs` |
| `CLOSE_BTN_MARGIN_LOGICAL` | 2.0 | `ui/src/widgets/list.rs` |
| `DEFAULT_TAB_WIDTH` | 4 | `ui/src/layout.rs` |
| `ATLAS_SIZE` | 2048 | `ui/src/gutter.rs` |
| `WINDOW_TITLE` | "edit+" | `app/src/app.rs` + `app/src/app_lifecycle.rs` ⚠️ 重复定义 |
| `OVERSCAN_ROWS` | 500 | `app/src/render_cache.rs` |
| `MAX_CACHED_LINES` | 1000 | `app/src/render_cache.rs` |
| `RECENT_SLOTS` | 20 | `app/src/native_menu.rs` |
| `MAX_ENTRIES` | 100 | `app/src/file_history.rs` |

---

### 15.2 散落的魔法数字（应提取为 const）

#### **A. UI 尺寸类**

| 值 | 出现位置 | 含义 | 建议常量名 |
|----|----------|------|-----------|
| `28.0` | sidebar `HEADER_H`, search_bar `SEARCH_BAR_HEIGHT`, sidebar `NEW_BTN_H`, sidebar `SETTINGS_BTN_H` | 通用 UI 条高度 | `BAR_HEIGHT`（已有，但分散在 4 处同值不同名） |
| `24.0` | sidebar `ROW_H` | 列表行高 | 已有 `ROW_H` |
| `220.0` | `sidebar.rs:27` | sidebar 默认宽度 | `SIDEBAR_DEFAULT_WIDTH` |
| `160.0` / `400.0` | `sidebar.rs:31-32, 440-441` | sidebar 最小/最大宽度 | `SIDEBAR_MIN_WIDTH` / `SIDEBAR_MAX_WIDTH` |
| `12.0` | sidebar padding、popup_menu text_x | 通用水平内边距 | `H_PADDING` |
| `8.0` | sidebar 间距多处、title_bar offset、popup_menu text_x | 通用小间距 | `SMALL_GAP` |
| `6.0` | sidebar `PADDING`、popup_menu padding | 内边距 | 已有 `PADDING` 但 popup_menu 重复硬编码 |
| `4.0` | popup_menu padding、scrollbar thumb idle | 内边距 | 已有 const 但 popup_menu 没用 |
| `2.0` | sidebar 间距、list widget margin | 微间距 | 已有 const 但 sidebar 没用 |
| `25.0` | `scrollbar.rs:52` | thumb 最小高度 | `MIN_THUMB_HEIGHT` |
| `14.0` | scrollbar reserve、popup_menu font_size | 滚动条宽度/字体 | 已有 `SCROLLBAR_RESERVE_PX` |
| `32.0` | `status_bar.rs` text left margin、`cursor_motion.rs:121` | 左侧边距 | `LEFT_MARGIN` 或 `GUTTER_PADDING` |
| `16.0` | sidebar `btn_size`、menu button size | 按钮尺寸 | `BUTTON_SIZE` |
| `10.0` | sidebar settings button margin、title_bar path gap | 间距 | `MEDIUM_GAP` |

#### **B. 字体大小类**

| 值 | 出现位置 | 含义 | 建议常量名 |
|----|----------|------|-----------|
| `14.0` | popup_menu font_size、search_bar font_size、shaping default | 正文字体 | `BODY_FONT_SIZE` |
| `13.0` | `title_bar.rs` name_font_size | 文件名字体 | `TITLE_FONT_SIZE` |
| `10.0` | `title_bar.rs` path_font_size、`status_bar.rs` font_size | 辅助/小字体 | `CAPTION_FONT_SIZE` |
| `15.0` | `settings.rs` default font_size | 编辑器默认字号 | 已有 `Settings::default()` |
| `0.8` | `render_pipeline.rs` ×4, `gutter.rs` ×1, `decorations.rs` ×1, `render_cache.rs` ×1 | 行号字号 = 正文字号 × 0.8 | `LN_FONT_SCALE` |

**`* 0.8` 出现了 7 次**（行号字号缩放比），分散在 3 个文件中：
```
render_pipeline.rs:40,281,417,711  → text.shaper.font_size() * 0.8
render_pipeline.rs:737,1006       → Settings::with(|s| s.line_height) * 0.8
render_cache.rs                   → line_height * 0.8
gutter.rs:70                      → settings_line_height * 0.8
decorations.rs:12                 → settings.line_height * 0.8
```

#### **C. 基线/排版比率类**

| 值 | 出现位置 | 含义 | 建议常量名 |
|----|----------|------|-----------|
| `0.8` | gutter `y_base`、render_pipeline `y_base`、render_cache、decorations | 基线位置 = 行高 × 0.8 | `BASELINE_RATIO` |
| `0.35` | `status_bar.rs` y_baseline offset | 文字垂直居中偏移 | `VCENTER_OFFSET_RATIO` |
| `0.6` | `title_bar.rs` y_baseline ratio | 文字垂直居中 | `TITLE_VCENTER_RATIO` |
| `0.65` | `popup_menu.rs` text_y ratio | 文字垂直居中 | `MENU_VCENTER_RATIO` |
| `-0.12` | `gutter.rs:63` letter_spacing | 行号字间距 | `GUTTER_LETTER_SPACING` |
| `0.75` | `render_pipeline.rs:1009` underline alpha | 下划线透明度 | `UNDERLINE_ALPHA` |

**问题：** 垂直居中比例有 3 个不同值（0.35、0.6、0.65），可能是因为不同字体基线差异，但应该统一计算方法。

#### **D. 颜色硬编码（popup_menu 内联 light theme）**

`ui/src/widgets/popup_menu.rs` 中有 27 个颜色值硬编码为 light theme fallback：

```rust
background: [0.93, 0.93, 0.95, 1.0],
menu_bg: [0.95, 0.95, 0.97, 1.0],
menu_border: [0.82, 0.82, 0.85, 1.0],
menu_hover: [0.88, 0.90, 0.96, 1.0],
// ... 等等共 27 个颜色
```

这些颜色与 `theme.rs` 的 dark theme 重复定义了同一套语义颜色。

**建议：** popup_menu widget 的测试不应自己构造 `test_theme()`，应该用 `Theme::light()` 或 `Theme::dark()`。同样 7 个文件各自有 `fn test_theme()` 共 7 份重复。

#### **E. App 层散落值**

| 值 | 出现位置 | 含义 | 建议常量名 |
|----|----------|------|-----------|
| `68.0 + 6.0 + 14.0 + 8.0` | `app_renderer.rs:351` | hamburger 按钮区域总宽（96.0） | `TRAFFIC_LIGHT_TOTAL_W` |
| `32.0` | `cursor_motion.rs:121` | 光标初始 x 偏移 | `GUTTER_LEFT_MARGIN` |
| `200.0` / `400.0` | `sidebar.rs:339, 440-441` test | sidebar 宽度 clamp 范围 | 已有 const 但测试中重复硬编码 |
| `1920.0 / 1080.0` | 测试中 | 测试屏幕尺寸 | `TEST_SCREEN_W / TEST_SCREEN_H` |

---

### 15.3 `test_theme()` 重复定义（7 份）

以下文件各有独立的 `fn test_theme() -> Theme`，内容基本相同（构造一个全零 Theme）：

| 文件 | 有效代码行 |
|------|-----------|
| `ui/src/widgets/status_bar.rs` | ~30 行 |
| `ui/src/widgets/title_bar.rs` | ~30 行 |
| `ui/src/widgets/popup_menu.rs` | ~30 行 |
| `ui/src/widgets/scrollbar.rs` | ~30 行 |
| `ui/src/widgets/sidebar.rs` | ~30 行 |
| `ui/src/widgets/search_bar.rs` | ~30 行 |
| `app/src/ui_shell.rs` | ~30 行 |

**合计 ~210 行重复测试代码。**

**建议：** 在 `ui/src/theme.rs` 中加 `#[cfg(test)] pub fn test_theme() -> Theme`，所有测试共用。

---

### 15.4 硬编码优先级汇总

| 优先级 | 项目 | 影响 |
|--------|------|------|
| P1 | `* 0.8` 基线/字号比（7 处分散） | 修改基线算法需改 7 处 |
| P1 | `test_theme()` 7 份重复（210 行） | 新增 Theme 字段需改 7 处 |
| P2 | `28.0` 高度值 4 处不同名常量 | 应统一为一个 `BAR_HEIGHT` |
| P2 | 垂直居中比例 0.35/0.6/0.65 三套 | 不一致容易出 bug |
| P2 | popup_menu 内联 light theme 27 色 | 应移入 `Theme::light()` |
| P2 | `WINDOW_TITLE` 重复定义 2 处 | 移入 workspace Cargo.toml 或 constants |
| P3 | sidebar 间距值 12/8/6/4/2 多处 | 已有 const 但未全部引用 |
| P3 | 字体大小 14/13/10 无常量名 | 定义 `BODY/TITLE/CAPTION_FONT_SIZE` |
| P3 | `hamburger_right = 68+6+14+8` 表达式 | 提取为 `TRAFFIC_LIGHT_TOTAL_W` |
