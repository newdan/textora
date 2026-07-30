# 大文件拆分实施计划 (app + document_view + sidebar)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development.

**Goal:** 拆分大文件 — app.rs (3481行), document_view/mod.rs (1275行), widgets/sidebar/types.rs (1795行), widgets/sidebar/mod.rs (1358行)

**Spec:** `docs/superpowers/specs/2026-06-14-multi-file-split-design.md`

**实施顺序:** app → document_view → sidebar

---

## Part A: app.rs 拆分（3481 → 目标 ~150）

### ✅ 已完成

| 文件 | 行数 | 内容 |
|------|------|------|
| `app_event.rs` | 7 | AppEvent 枚举 |
| `app_init.rs` | 262 | App::new() 初始化 |
| `app_lifecycle.rs` | 416 | ApplicationHandler<AppEvent> for App |
| `app_renderer.rs` | 608 | editor_left_margin, cursor_vertices, 顶点生成/GPU 提交 |
| `app_tests.rs` | 1121 | 测试代码 |

lib.rs 已有: `pub mod app_event`, `pub mod app_init`, `pub mod app_lifecycle`, `pub mod app_renderer`

### ❌ 待完成 — 原始计划中 6 个功能文件均未创建

#### A.1 创建 `app_tab.rs`（最高优先）

包含方法（均在 app.rs 第一个 impl 块）：
- `config_dir`, `record_tab_to_history`, `record_all_tabs_to_history`, `save_history`
- `update_document_edited`, `handle_workspace_effect`, `update_window_title`
- `update_tab_layout`, `open_file`, `open_file_dialog`
- `new_empty_tab`, `try_close_tab_with_prompt`, `try_close_multiple_with_prompt`, `execute_batch_close`
- `load_file`
- `save_workspace_snapshot`

#### A.2 创建 `app_dispatch.rs`

- `execute_commands`, `dispatch_menu_action`, `dispatch`
- `handle_sidebar_key_action`, `handle_command`
- 测试: edit_command_tests, sidebar_integration_tests

#### A.3 创建 `app_reshape.rs`

- `invalidate_reshape`, `apply_zoom`
- `drain_reshape_results`, `submit_reshape_ahead`, `post_shape_update`
- 测试: zoom_tests

#### A.4 创建 `app_scroll.rs`

- `move_cursor_visual`, `page_up`, `page_down`
- `extend_selection_visual`, `handle_scroll`

#### A.5 创建 `app_window.rs`

- `build_shell_inputs`, `quit_app`, `save_window_geometry`
- `update_ime_cursor_area`, `flush_pending_resize`, `handle_resize`, `resize`
- `has_active_animation`, `compute_next_wake_time`
- 测试: build_shell_inputs_tests, ime_preedit_tests

#### A.6 创建 `app_search.rs`

- `perform_search_for_active_doc`, `apply_search_bar_action`, `scroll_to_active_match`

#### A.7 精简 app.rs

仅保留: App struct 定义, `compute_cursor_phase`, `reset_after_edit`, 纯 getter 方法

### 验证

```bash
cargo build -p app && cargo test -p app
```

---

## Part B: document_view/mod.rs 拆分（1275 → 目标 ~700）

### ✅ 已完成

| 文件 | 行数 | 内容 |
|------|------|------|
| `cursor.rs` | 32 | CursorState 结构体 |
| `display.rs` | 33 | DisplayState 结构体 |
| `*_tests.rs` (7个) | ~2200 | 测试文件已分离 |

### ❌ 待完成

#### B.1 创建 `document_view/visible.rs`

移走 visible_* 方法（mod.rs ln~250-391）:
- `visible_doc_range` → `pub(crate) fn`
- `visible_line`, `visible_line_wrap`, `visible_lines`, `visible_line_count`
- `visible_line_count_wrap`, `visible_line_key`, `visible_line_key_wrap`

#### B.2 创建 `document_view/selection.rs`

移走选区/剪贴板方法（mod.rs ln~698-969）:
- `word_select_at`, `extract_selected_text`, `count_selection_chars`
- `paste_text`, `copy_selection_to_clipboard`, `cut_selection_to_clipboard`, `paste_from_clipboard`
- `ensure_selection_active`, `extend_selection_*` (12个), `has_selection`, `selection_range`
- `clear_selection`, `select_all`, `delete_selection`

#### B.3 创建 `document_view/edit.rs`

移走编辑/undo/sync 方法（mod.rs ln~463-1120）:
- `insert_at_cursor`, `delete_backward`, `delete_forward`, `undo`, `redo`
- `sync_after_edit_incremental_undo_redo`, `sync_after_edit_incremental`
- `sync_cursor`, `set_cursor_offset_synced`, `sync_cursor_offset_from_tb`, `assert_cursor_synced`

#### B.4 保留在 mod.rs

DocumentView struct 定义, 构造/查询/光标移动/搜索/布局方法, `replace_null_bytes`, `normalize_paste_text`

### 验证

```bash
cargo build -p app && cargo test -p app
```

---

## Part C: widgets/sidebar/ 拆分

### 当前状态

| 文件 | 行数 | 测试数 | 内容 |
|------|------|--------|------|
| `mod.rs` | 1358 | 37 | SidebarWidget struct + Widget trait impl + tests |
| `types.rs` | 1795 | 56 | 所有类型定义 + SidebarState impl(3块) + build_settings_menu + tests |
| `persistent.rs` | 159 | 0 | SidebarPersistent（已合理，不拆分） |

### types.rs 内部结构

| 区域 | 行号 | 行数 | 内容 |
|------|------|------|------|
| 类型定义 | 1-151 | ~150 | SidebarConfig, Visibility, SidebarInput, SidebarKey, SidebarAction, SidebarLayoutItem, SidebarLayout, EdgeDragState, SidebarHoverButton, SidebarState struct |
| impl 块1 | 153-272 | ~120 | new, visibility getter/setter, persist, scroll, current_width, editor_left_offset, is_visible |
| 常量 | 274-284 | ~11 | HOT_BAND_LOGICAL, HEADER_H, ROW_H, etc. |
| impl 块2 | 286-535 | ~250 | on_drag_start, on_drag, on_drag_end, open_settings_menu, dispatch_menu_click, update_layout |
| impl 块3 | 537-897 | ~361 | on_mouse_move, tick, on_key, paint, hit_test_px |
| 辅助函数 | 904-947 | ~44 | build_settings_menu |
| 测试 | 949-1795 | ~846 | 56 个测试 |

### mod.rs 内部结构

| 区域 | 行号 | 行数 | 内容 |
|------|------|------|------|
| 模块声明+re-export | 1-11 | ~11 | |
| SidebarWidget struct | 17-52 | ~36 | 结构体定义 |
| make_style_from_theme | 54-68 | ~15 | 辅助函数 |
| impl SidebarWidget | 70-254 | ~185 | new, set_input, 委托方法 |
| impl Widget | 256-574 | ~319 | set_rect, paint, hit, on_event |
| 测试 | 577-1358 | ~782 | 37 个测试 |

### 拆分方案

```
widgets/sidebar/
├── mod.rs          (~600行) 模块声明 + re-export + SidebarWidget + Widget impl
├── types.rs        (~180行) 纯类型定义（无 SidebarState，无测试）
├── layout.rs       (~30行)  SidebarLayoutItem + SidebarLayout（从 types.rs 移出）
├── state.rs        (~750行) SidebarState struct + 3个 impl 块 + 常量
├── menu.rs         (~100行) open_settings_menu (SidebarState方法) + build_settings_menu (自由函数)
├── persistent.rs   (~159行) 不变
├── widget_tests.rs (~780行) mod.rs 的 Widget 测试移入
└── state_tests.rs  (~850行) types.rs 的 SidebarState 测试移入
```

#### C.1 创建 `layout.rs`

从 types.rs 移出 SidebarLayoutItem + SidebarLayout（行 97-116）:

```rust
//! Sidebar layout types.

use crate::core::Rect;
use crate::tab_bar::TabIndicator;

#[derive(Debug, Clone)]
pub struct SidebarLayoutItem {
    pub tab_index: usize,
    pub rect: Rect,
    pub title: String,
    pub indicator: TabIndicator,
}

#[derive(Debug, Clone, Default)]
pub struct SidebarLayout {
    pub bg_rect: Rect,
    pub header_rect: Rect,
    pub menu_btn_rect: Rect,
    pub new_btn_rect: Rect,
    pub items: Vec<SidebarLayoutItem>,
    pub files_header_rect: Rect,
    pub list_clip: Rect,
    pub settings_btn_rect: Rect,
    pub edge_resize_rect: Rect,
}
```

#### C.2 简化 `types.rs`（~180行，无测试）

仅保留:
- SidebarConfig + impl (clamp_width)
- Visibility + Default
- SidebarInput, SidebarKey, SidebarAction
- SidebarHoverButton
- HOT_BAND_LOGICAL 常量
- EdgeDragState（私有 struct，仅 state.rs 使用，可移入 state.rs）

types.rs 不再包含 SidebarState、build_settings_menu 或任何测试。

#### C.3 创建 `state.rs`（~750行，含测试 ~850行）

从 types.rs 移入:
- SidebarState struct 定义（行 136-151）
- impl SidebarState 块 1（行 153-272）：构造 + 基础方法
- 布局常量（行 274-284）：HEADER_H, ROW_H, etc.
- impl SidebarState 块 2（行 286-535）：drag + settings_menu + update_layout
- impl SidebarState 块 3（行 537-897）：on_mouse_move + tick + on_key + paint + hit_test_px
- EdgeDragState（从 types.rs 移入，仅 state 内部使用）
- 测试模块（行 949-1795）：56 个测试

```rust
//! SidebarState — main state machine for sidebar.

use std::time::Instant;
use crate::core::{Rect, PaintCtx};
use crate::settings::Settings;
use crate::widgets::popup_menu::{PopupMenu, PopupMenuAction as PMA, PopupMenuItem};
use crate::view_mode::ViewMode;
use crate::sidebar::types::*;
use crate::sidebar::layout::*;
use crate::sidebar::persistent::SidebarPersistent;

struct EdgeDragState {
    start_px: f32,
    start_width: f32,
}

#[derive(Default)]
pub struct SidebarState {
    // fields...
}

impl SidebarState {
    // block 1: construction + basic methods
}

impl SidebarState {
    // block 2: drag + settings menu + layout
    pub fn open_settings_menu(...)
    pub fn dispatch_menu_click(...)
}

impl SidebarState {
    // block 3: mouse_move + tick + key + paint + hit_test
}

#[cfg(test)]
mod tests {
    // 56 tests from types.rs
}
```

注意: `open_settings_menu` 和 `dispatch_menu_click` 是 `SidebarState` 的方法，保留在 state.rs 的 impl 块中，不单独拆文件。

#### C.4 创建 `menu.rs`（~55行）

仅包含 `build_settings_menu` 自由函数（行 904-947），该函数不依赖 SidebarState:

```rust
//! Settings menu builder — standalone free function.

use crate::core::Rect;
use crate::widgets::popup_menu::{PopupMenu, PopupMenuItem, PopupMenuAction as PMA};
use crate::constants;
use crate::view_mode::ViewMode;

pub fn build_settings_menu(
    _layout: Option<&crate::sidebar::layout::SidebarLayout>,
    screen_w: f32,
    screen_h: f32,
) -> Option<PopupMenu> {
    // ... (copy from types.rs lines 904-947)
}
```

#### C.5 精简 `mod.rs`（~600行）

从 mod.rs 移出测试模块到 `widget_tests.rs`:

```rust
//! SidebarWidget — merged from old ui/src/sidebar.rs + ui/src/widgets/sidebar.rs.

pub mod types;
pub mod layout;
pub mod persistent;
pub mod state;
pub mod menu;

// Re-export all public types for backward compatibility
pub use types::{
    SidebarAction, SidebarConfig, SidebarHoverButton, SidebarInput, SidebarKey, Visibility,
};
pub use layout::{SidebarLayout, SidebarLayoutItem};
pub use persistent::SidebarPersistent;
pub use state::SidebarState;
pub use menu::build_settings_menu;

// SidebarWidget struct + impl block + Widget impl (lines 17-574)
// 注意: use 路径更新
use crate::sidebar::types::*;
use crate::sidebar::layout::*;
use crate::sidebar::state::SidebarState;
// ...

#[cfg(test)]
mod tests {
    // 移入 widget_tests.rs，或保留 0 测试
}
```

#### C.6 创建 `widget_tests.rs`（~780行）

从 mod.rs 行 576-1358 移入 37 个 Widget 测试。

```rust
//! SidebarWidget tests.

#[cfg(test)]
mod tests {
    use crate::sidebar::{SidebarWidget, SidebarConfig, SidebarAction, Visibility};
    // ... 37 tests
}
```

### C.7 更新外部引用

需要检查并更新以下位置的 use 路径:
- `crates/ui/src/widgets/sidebar/mod.rs` — use self::types::* → use crate::sidebar::types::*
- `crates/app/src/` 中使用 `ui::widgets::sidebar::*` 的文件 — 检查 pub use re-export 是否覆盖
- `crates/ui/src/` 内的其他文件 — 同理

### C.8 删除旧内容

- 从 types.rs 删除：SidebarState、build_settings_menu、所有测试、SidebarLayout/SidebarLayoutItem
- 从 mod.rs 删除：测试模块

### 验证

```bash
cargo build --workspace && cargo test --workspace
```

预期: 编译成功，93 个测试全部通过（56 state + 37 widget）。

---

## 总结

### 实施顺序

1. **Part C (sidebar)** — 影响面最小，纯模块内部重组，re-export 保证兼容
2. **Part B (document_view)** — 中等影响，3 个 impl 块分文件
3. **Part A (app)** — 最大工作量，6 个新文件

### 目标行数

| 文件 | 当前 | 目标 |
|------|------|------|
| `app.rs` | 3481 | ≤150 |
| `document_view/mod.rs` | 1275 | ≤700 |
| `sidebar/types.rs` | 1795 | ≤180 |
| `sidebar/mod.rs` | 1358 | ≤600 |
| `sidebar/state.rs` | 新 | ≤800 |
| `sidebar/widget_tests.rs` | 新 | ≤780 |
| `sidebar/state_tests.rs` | 新 | ≤850 |

### 全量验证

```bash
# 编译
cargo build --workspace

# 测试
cargo test --workspace

# Clippy
cargo clippy --workspace -- -A clippy::all 2>&1 | grep -E "warning|error"

# 行数统计
wc -l crates/app/src/app.rs \
      crates/app/src/app_*.rs \
      crates/app/src/document_view/{mod,visible,selection,edit,cursor,display}.rs \
      crates/app/src/document_view/*_tests.rs \
      crates/ui/src/widgets/sidebar/{mod,types,layout,state,menu,persistent}.rs \
      crates/ui/src/widgets/sidebar/*_tests.rs
```
