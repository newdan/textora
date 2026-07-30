# app.rs 拆分实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `crates/app/src/app.rs`（3328行）拆分为 7 个文件：1 个薄壳 app.rs + 6 个领域文件。

**Architecture:** 利用 Rust 跨文件 `impl App` 机制（已有先例 `app_lifecycle.rs`、`app_renderer.rs`、`app_init.rs`），按职责域分配方法。每个文件底部保留 `#[cfg(test)]` 测试。

**Tech Stack:** Rust, winit, wgpu

**Spec:** `docs/superpowers/specs/2026-06-14-app-rs-split-design.md`

---

## 文件结构总览

```
crates/app/src/
├── app.rs              # (~120行) 薄壳：struct + 纯 getter + 自由函数
├── app_search.rs       # (~80行)  perform_search_for_active_doc
├── app_reshape.rs      # (~200行) Reshape管线 + apply_zoom
├── app_scroll.rs       # (~180行) 滚动 + 光标视觉移动
├── app_tab.rs          # (~350行) Tab/文件/历史/workspace联动
├── app_window.rs       # (~170行) 窗口初始化/缩放/标题/build_shell_inputs
├── app_dispatch.rs     # (~300行) 命令路由 (dispatch/execute/handle_command)
│
├── app_lifecycle.rs    # (已有) ApplicationHandler impl
├── app_init.rs         # (已有) init_display_map
├── app_renderer.rs     # (已有) render
└── lib.rs              # 追加 6 个 pub mod 声明
```

### 依赖关系

```
app_search.rs  ← 无内部依赖
app_reshape.rs ← 无内部依赖
app_scroll.rs  ← 仅访问 workspace 字段
app_tab.rs     ← 调用 invalidate_reshape/update_tab_layout
app_window.rs  ← 调用 invalidate_reshape/build_shell_inputs
app_dispatch.rs ← 调用以上所有
```

---

### Task 1: 创建 `app_search.rs`（最独立）

**Files:**
- Create: `crates/app/src/app_search.rs`
- Modify: `crates/app/src/lib.rs`

**生产代码：** 从 app.rs 行 1806-1872 移动 `perform_search_for_active_doc`

- [ ] **Step 1: 创建 app_search.rs**

```rust
//! 搜索逻辑 — 在活动文档中搜索。

use crate::App;

impl App {
    pub(crate) fn perform_search_for_active_doc(&mut self) {
        // === 复制 app.rs 行 1806-1872 的方法体 ===
        if let Some(dv) = self.workspace.doc_views.get_mut(self.workspace.active_index) {
            let query = dv.search_state.query.clone();
            if query.is_empty() {
                dv.search_state.matches.clear();
                dv.search_state.active_match_idx = 0;
                dv.search_state.buffer_generation = dv.tb.gap_buffer().generation();
                return;
            }

            use core::document::ReadableDocument;
            let chunk1 = dv.tb.gap_buffer().read_forward(0);
            let chunk2 = dv.tb.gap_buffer().read_forward(chunk1.len());

            let query_bytes = query.as_bytes();
            let search_fn: fn(&[u8], &[u8]) -> Vec<std::ops::Range<usize>> =
                if dv.search_state.options.match_case {
                    core::buffer::simd_search::find_all
                } else {
                    core::buffer::simd_search::find_all_case_insensitive_ascii
                };

            let mut matches = Vec::new();

            if !chunk1.is_empty() {
                matches.extend(search_fn(query_bytes, chunk1));
            }

            if !chunk1.is_empty() && !chunk2.is_empty() && query_bytes.len() > 1 {
                let cross_len = query_bytes.len() - 1;
                let take1 = cross_len.min(chunk1.len());
                let take2 = cross_len.min(chunk2.len());

                let mut cross_buf = Vec::with_capacity(take1 + take2);
                cross_buf.extend_from_slice(&chunk1[chunk1.len() - take1..]);
                cross_buf.extend_from_slice(&chunk2[..take2]);

                let cross_matches = search_fn(query_bytes, &cross_buf);
                for m in cross_matches {
                    let start_in_doc = chunk1.len() - take1 + m.start;
                    matches.push(start_in_doc..start_in_doc + query_bytes.len());
                }
            }

            if !chunk2.is_empty() {
                let m2 = search_fn(query_bytes, chunk2);
                for m in m2 {
                    matches.push(m.start + chunk1.len()..m.end + chunk1.len());
                }
            }

            let generation = dv.tb.gap_buffer().generation();
            dv.search_state.update_matches(matches, generation);

            if dv.search_state.active_match().is_some() {
                let range = dv.search_state.matches[0].clone();
                dv.set_cursor_offset_synced(range.start);
                dv.cursor_mut().selection_anchor = Some(range.start);
                dv.set_cursor_offset_synced(range.end);
            }
        }
    }
}
```

- [ ] **Step 2: 在 lib.rs 中声明模块**

Edit `crates/app/src/lib.rs`，在合适位置添加（按字母序，在 `app_renderer` 之后）：

```rust
pub mod app_search;
```

- [ ] **Step 3: 编译验证**

```bash
cargo build -p app 2>&1 | head -20
```

预期：可能有 unused import 警告，但不应该有编译错误。如果 `perform_search_for_active_doc` 仍在 app.rs 中会造成重复定义，先保留 app.rs 中的方法，不移走。

---

### Task 2: 创建 `app_reshape.rs`

**Files:**
- Create: `crates/app/src/app_reshape.rs`
- Modify: `crates/app/src/lib.rs`

**生产代码：** 
- `invalidate_reshape` (236-242)
- `apply_zoom` (243-266)
- `drain_reshape_results` (1495-1564)
- `submit_reshape_ahead` (1565-1642)
- `post_shape_update` (1643-1684)

**测试代码：** zoom_tests module (2260-2621)

- [ ] **Step 1: 创建 app_reshape.rs（生产代码）**

```rust
//! Reshape 管线 + 缩放。

use std::collections::HashSet;
use std::sync::Arc;

use crate::reshape_worker::ReshapeRequest;
use crate::App;
use ui::settings::Settings;

impl App {
    pub(crate) fn invalidate_reshape(&mut self) {
        // === 复制 app.rs 行 236-242 的方法体 ===
        self.reshape_generation += 1;
        self.pending_reshapes.clear();
        if let Some(w) = &self.reshape_worker {
            w.cancel_before(self.reshape_generation);
        }
        self.frame_cache.advance_cache.clear();
        self.frame_cache.cluster_pool.clear();
    }

    pub(crate) fn apply_zoom(&mut self, font_size: f32) {
        // === 复制 app.rs 行 243-266 的方法体 ===
        let font_size = font_size.clamp(6.0, 48.0);
        Settings::with_mut(|s| {
            s.font_size = font_size;
            s.line_height = font_size * 1.6;
        });
        // ... (完整方法体从 app.rs 复制)
    }

    pub(crate) fn drain_reshape_results(&mut self) {
        // === 复制 app.rs 行 1495-1564 的方法体 ===
        // ... (完整方法体)
    }

    pub(crate) fn submit_reshape_ahead(&mut self) {
        // === 复制 app.rs 行 1565-1642 的方法体 ===
        // ... (完整方法体)
    }

    pub(crate) fn post_shape_update(&mut self) {
        // === 复制 app.rs 行 1643-1684 的方法体 ===
        // ... (完整方法体)
    }
}

#[cfg(test)]
mod zoom_tests {
    use super::*;
    use crate::render_cache::{CachedLine, GlyphInstance};

    // === 复制 app.rs 行 2260-2621 的所有测试 ===
    // 包含: invalidate_reshape_*, sim_zoom_*, apply_zoom_*, zoom_* 等测试
}
```

- [ ] **Step 2: 在 lib.rs 中声明**

```rust
pub mod app_reshape;
```

- [ ] **Step 3: 编译验证**

```bash
cargo build -p app 2>&1 | head -20
```

---

### Task 3: 创建 `app_scroll.rs`

**Files:**
- Create: `crates/app/src/app_scroll.rs`

**生产代码：**
- `move_cursor_visual` (1403-1436)
- `page_up` (1437-1444)
- `page_down` (1445-1452)
- `extend_selection_visual` (1453-1494)
- `handle_scroll` (1723-1805)

> 注：这些都是 `fn`（私有），但因为跨文件需要改为 `pub(crate) fn`

- [ ] **Step 1: 创建 app_scroll.rs**

```rust
//! 滚动处理 + 光标视觉移动。

use winit::event::MouseScrollDelta;

use crate::cursor_motion::CursorContext;
use crate::document_view::DocumentView;
use crate::App;
use ui::settings::Settings;
use ui::tab_bar;

impl App {
    // === 复制 app.rs 行 1403-1436: move_cursor_visual ===
    pub(crate) fn move_cursor_visual(&mut self, delta: isize) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1437-1444: page_up ===
    pub(crate) fn page_up(&mut self) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1445-1452: page_down ===
    pub(crate) fn page_down(&mut self) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1453-1494: extend_selection_visual ===
    pub(crate) fn extend_selection_visual(&mut self, delta: isize) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1723-1805: handle_scroll ===
    pub(crate) fn handle_scroll(&mut self, delta: MouseScrollDelta) {
        // 方法体从 app.rs 复制
    }
}
```

- [ ] **Step 2: 在 lib.rs 中声明**

```rust
pub mod app_scroll;
```

- [ ] **Step 3: 编译验证**

```bash
cargo build -p app 2>&1 | head -30
```

---

### Task 4: 创建 `app_tab.rs`

**Files:**
- Create: `crates/app/src/app_tab.rs`

**生产代码：** 最多方法的一个文件。
- `save_workspace_snapshot` (267-270)
- `record_tab_to_history` (300-309)
- `record_all_tabs_to_history` (310-334)
- `save_history` (335-341)
- `update_document_edited` (342-365)
- `handle_workspace_effect` (366-399)
- `update_window_title` (400-416)
- `update_tab_layout` (421-437)
- `open_file` (438-444)
- `open_file_dialog` (445-522)
- `new_empty_tab` (523-529)
- `try_close_tab_with_prompt` (530-619)
- `try_close_multiple_with_prompt` (620-710)
- `execute_batch_close` (711-735)
- `config_dir` (294-299)
- `load_file` (1312-1332)

注意：
- `config_dir` 需要改为 `pub(crate) fn`（从 `fn`）
- `save_window_geometry` 移入 `app_window.rs`
- `handle_workspace_effect` 是核心方法，调用 `invalidate_reshape` (在 app_reshape.rs)、`update_tab_layout` (在同一文件) 等

- [ ] **Step 1: 创建 app_tab.rs**

```rust
//! Tab 管理 + 文件操作 + 历史记录。

use std::path::PathBuf;

use crate::file_history::{FileHistoryEntry, compute_workspace_root};
use crate::workspace::WorkspaceEffect;
use crate::App;
use ui::settings::Settings;
use ui::tab_bar;

impl App {
    // === 复制 app.rs 行 294-299: config_dir ===
    pub(crate) fn config_dir() -> PathBuf {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 300-309: record_tab_to_history ===
    fn record_tab_to_history(&mut self, index: usize) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 310-334: record_all_tabs_to_history ===
    fn record_all_tabs_to_history(&mut self) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 335-341: save_history ===
    fn save_history(&self) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 342-365: update_document_edited ===
    pub(crate) fn update_document_edited(&self, edited: bool) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 366-399: handle_workspace_effect ===
    pub(crate) fn handle_workspace_effect(&mut self, effect: WorkspaceEffect) {
        // 方法体从 app.rs 复制
        // 注意：此方法调用 self.invalidate_reshape() 和 self.init_display_map()
        // 这些方法在其他 impl App 文件中，Rust 会自动找到它们
    }

    // === 复制 app.rs 行 400-416: update_window_title ===
    pub(crate) fn update_window_title(&self) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 267-270: save_workspace_snapshot ===
    pub(crate) fn save_workspace_snapshot(&self) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 421-437: update_tab_layout ===
    pub(crate) fn update_tab_layout(&mut self, autoscroll: bool) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 438-444: open_file ===
    pub(crate) fn open_file(&mut self, path: &std::path::Path) -> Result<(), String> {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 445-522: open_file_dialog ===
    pub(crate) fn open_file_dialog(&mut self) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 523-529: new_empty_tab ===
    pub(crate) fn new_empty_tab(&mut self) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 530-619: try_close_tab_with_prompt ===
    pub(crate) fn try_close_tab_with_prompt(&mut self, idx: usize) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 620-710: try_close_multiple_with_prompt ===
    pub(crate) fn try_close_multiple_with_prompt(
        &mut self,
        action: ui::tab_bar::ContextMenuAction,
        idx: usize,
    ) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 711-735: execute_batch_close ===
    fn execute_batch_close(&mut self, indices: &[usize]) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1312-1332: load_file ===
    fn load_file(&mut self) {
        // 方法体从 app.rs 复制
    }
}
```

- [ ] **Step 2: 在 lib.rs 中声明**

```rust
pub mod app_tab;
```

- [ ] **Step 3: 编译验证**

```bash
cargo build -p app 2>&1 | head -30
```

---

### Task 5: 创建 `app_window.rs`

**Files:**
- Create: `crates/app/src/app_window.rs`

**生产代码：**
- `build_shell_inputs` (177-231)
- `save_window_geometry` (281-293)
- `quit_app` (271-280)
- `update_ime_cursor_area` (1187-1215)
- `init_window` (1216-1311)
- `flush_pending_resize` (1333-1345)
- `handle_resize` (1346-1351)
- `resize` (1352-1402)
- `has_active_animation` (1685-1691)
- `compute_next_wake_time` (1692-1722)

**测试代码：**
- `ime_tests` (2975-3028)
- `layout_alignment_tests` (3029-3100)
- `build_shell_inputs_tests` (3101-3209)
- `ui_shell_basic_tests` (3287-3328)

- [ ] **Step 1: 创建 app_window.rs**

```rust
//! 窗口管理 + shell 布局 + IME。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use winit::dpi::PhysicalSize;
use winit::window::WindowAttributes;

use crate::gpu::GpuError;
use crate::ui_shell::ShellInputs;
use crate::App;
use ui::settings::Settings;

impl App {
    // === 复制 app.rs 行 177-231: build_shell_inputs ===
    pub(crate) fn build_shell_inputs(&self) -> ShellInputs {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 271-280: quit_app ===
    pub(crate) fn quit_app(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 281-293: save_window_geometry ===
    fn save_window_geometry(&mut self) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1187-1215: update_ime_cursor_area ===
    pub(crate) fn update_ime_cursor_area(&self) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1216-1311: init_window ===
    pub(crate) fn init_window(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) -> Result<(), GpuError> {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1333-1345: flush_pending_resize ===
    pub(crate) fn flush_pending_resize(&mut self) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1346-1351: handle_resize ===
    pub fn handle_resize(&mut self, width: u32, height: u32) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1352-1402: resize ===
    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1685-1691: has_active_animation ===
    pub(crate) fn has_active_animation(&self) -> bool {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1692-1722: compute_next_wake_time ===
    pub(crate) fn compute_next_wake_time(&self) -> Option<Instant> {
        // 方法体从 app.rs 复制
    }
}

#[cfg(test)]
mod ime_tests {
    use super::*;
    // === 复制 app.rs 行 2975-3028 的所有测试 ===
}

#[cfg(test)]
mod layout_alignment_tests {
    use super::*;
    // === 复制 app.rs 行 3029-3100 的所有测试 ===
}

#[cfg(test)]
mod build_shell_inputs_tests {
    use super::*;
    // === 复制 app.rs 行 3101-3209 的所有测试 ===
}

#[cfg(test)]
mod ui_shell_basic_tests {
    use super::*;
    // === 复制 app.rs 行 3287-3328 的所有测试 ===
}
```

- [ ] **Step 2: 在 lib.rs 中声明**

```rust
pub mod app_window;
```

- [ ] **Step 3: 编译验证**

```bash
cargo build -p app 2>&1 | head -30
```

---

### Task 6: 创建 `app_dispatch.rs`

**Files:**
- Create: `crates/app/src/app_dispatch.rs`

**生产代码：**
- `execute_commands` (736-904)
- `dispatch_menu_action` (905-913)
- `dispatch` (914-1161)
- `handle_sidebar_key_action` (1162-1186)
- `handle_command` (1873-2267)

**测试代码：**
- `edit_command_tests` (2622-2664)
- `sidebar_integration_tests` (2665-2974)
- `editor_left_margin_tests` (3210-3286)

- [ ] **Step 1: 创建 app_dispatch.rs**

```rust
//! 命令路由 — dispatch / execute_commands / handle_command。

use crate::actions::AppAction;
use crate::input::EditCommand;
use crate::App;
use ui::tab_bar;

impl App {
    // === 复制 app.rs 行 736-904: execute_commands ===
    pub(crate) fn execute_commands(
        &mut self,
        cmds: Vec<crate::menu_handler::AppCommand>,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 905-913: dispatch_menu_action ===
    pub(crate) fn dispatch_menu_action(
        &mut self,
        action: crate::native_menu::MenuAction,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 914-1161: dispatch ===
    pub(crate) fn dispatch(
        &mut self,
        action: AppAction,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1162-1186: handle_sidebar_key_action ===
    fn handle_sidebar_key_action(&mut self, action: ui::sidebar::SidebarAction) {
        // 方法体从 app.rs 复制
    }

    // === 复制 app.rs 行 1873-2267: handle_command ===
    pub(crate) fn handle_command(
        &mut self,
        cmd: EditCommand,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        // 方法体从 app.rs 复制
    }
}

#[cfg(test)]
mod edit_command_tests {
    use super::*;
    // === 复制 app.rs 行 2622-2664 的所有测试 ===
}

#[cfg(test)]
mod sidebar_integration_tests {
    use super::*;
    // === 复制 app.rs 行 2665-2974 的所有测试 ===
}

#[cfg(test)]
mod editor_left_margin_tests {
    use super::*;
    // === 复制 app.rs 行 3210-3286 的所有测试 ===
}
```

- [ ] **Step 2: 在 lib.rs 中声明**

```rust
pub mod app_dispatch;
```

- [ ] **Step 3: 编译验证**

```bash
cargo build -p app 2>&1 | head -30
```

---

### Task 7: 精简 `app.rs` 为薄壳

**Files:**
- Modify: `crates/app/src/app.rs`

此时 6 个新文件都已编译通过（方法在两个文件重复定义，Rust 会报 duplicate definition 错误）。

- [ ] **Step 1: 删除已移走的方法，精简 app.rs**

从 app.rs 中删除以下行范围的内容：

| 删除范围 | 内容 | 已移至 |
|----------|------|--------|
| 177-231 | `build_shell_inputs` | app_window.rs |
| 236-266 | `invalidate_reshape` + `apply_zoom` | app_reshape.rs |
| 267-270 | `save_workspace_snapshot` | app_tab.rs |
| 271-280 | `quit_app` | app_window.rs |
| 281-293 | `save_window_geometry` | app_window.rs |
| 294-299 | `config_dir` | app_tab.rs |
| 300-399 | `record_tab_*` + `save_history` + `update_document_edited` + `handle_workspace_effect` | app_tab.rs |
| 400-416 | `update_window_title` | app_tab.rs |
| 421-437 | `update_tab_layout` | app_tab.rs |
| 438-444 | `open_file` | app_tab.rs |
| 445-522 | `open_file_dialog` | app_tab.rs |
| 523-529 | `new_empty_tab` | app_tab.rs |
| 530-735 | `try_close_tab_*` + `execute_batch_close` | app_tab.rs |
| 736-904 | `execute_commands` | app_dispatch.rs |
| 905-913 | `dispatch_menu_action` | app_dispatch.rs |
| 914-1161 | `dispatch` | app_dispatch.rs |
| 1162-1186 | `handle_sidebar_key_action` | app_dispatch.rs |
| 1187-1215 | `update_ime_cursor_area` | app_window.rs |
| 1216-1311 | `init_window` | app_window.rs |
| 1312-1332 | `load_file` | app_tab.rs |
| 1333-1402 | `flush_pending_resize` + `handle_resize` + `resize` | app_window.rs |
| 1403-1494 | `move_cursor_visual` + `page_up/down` + `extend_selection_visual` | app_scroll.rs |
| 1495-1684 | `drain_reshape_*` + `submit_reshape_*` + `post_shape_update` | app_reshape.rs |
| 1685-1722 | `has_active_animation` + `compute_next_wake_time` | app_window.rs |
| 1723-1805 | `handle_scroll` | app_scroll.rs |
| 1806-1872 | `perform_search_for_active_doc` | app_search.rs |
| 1873-2267 | `handle_command` | app_dispatch.rs |
| 2260-3328 | 所有 `#[cfg(test)]` 块 | 各个新文件 |

- [ ] **Step 2: 精简 app.rs 的 imports**

删除不再需要的 imports，只保留：
```rust
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use winit::dpi::PhysicalPosition;
use winit::event_loop::EventLoopProxy;
use winit::window::Window;

use crate::frame_cache::FrameCache;
use crate::gpu;
use crate::mouse::MouseState;
use crate::reshape_worker::ReshapeWorker;
use crate::workspace::Workspace;
use crate::ui_shell::UiShell;
use crate::file_history::FileHistory;
use crate::document_view::DocumentView;
use ui::settings::Settings;
use ui::theme::Theme;
use ui::core::widget::KeyCode;
```

- [ ] **Step 3: 精简后的 app.rs 结构**

```rust
//! Application state — struct 定义 + 纯 getter + 自由函数。
//!
//! 具体领域方法分布在:
//!   app_tab.rs, app_dispatch.rs, app_scroll.rs,
//!   app_search.rs, app_reshape.rs, app_window.rs

use crate::actions::AppAction;
use crate::app_event::AppEvent;
use crate::gpu;

pub(crate) use crate::render_state::{ATLAS_SIZE, GpuState, TextState};

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::EventLoopProxy;
use winit::window::Window;

use crate::document_view::DocumentView;
use crate::file_history::FileHistory;
use crate::frame_cache::FrameCache;
use crate::mouse::MouseState;
use crate::native_menu::NativeMenu;
use crate::reshape_worker::ReshapeWorker;
use crate::ui_shell::{ShellInputs, UiShell};
use crate::workspace::Workspace;
use ui::settings::Settings;
use ui::theme::Theme;

const WINDOW_TITLE: &str = "edit+";

// ── 自由函数 ──

pub(crate) fn compute_cursor_phase(cursor_blink_instant: Instant) -> (bool, Instant) {
    let elapsed_ms = cursor_blink_instant.elapsed().as_millis() as u64;
    let period_ms: u64 = 500;
    let phase_in_period = elapsed_ms % (period_ms * 2);
    let currently_visible = phase_in_period < period_ms;
    let next_transition_ms = if currently_visible {
        period_ms - phase_in_period
    } else {
        period_ms * 2 - phase_in_period
    };
    let next_deadline = Instant::now() + Duration::from_millis(next_transition_ms + 5);
    (currently_visible, next_deadline)
}

pub(crate) fn reset_after_edit(
    generation: &mut u64,
    pending_reshapes: &mut HashSet<usize>,
    reshape_worker: &Option<ReshapeWorker>,
    cursor_render_state: &mut crate::cursor_motion::CursorRenderState,
) {
    *generation += 1;
    pending_reshapes.clear();
    if let Some(w) = reshape_worker {
        w.cancel_before(*generation);
    }
    cursor_render_state.sticky_x_dirty = true;
    cursor_render_state.cursor_blink_instant = Instant::now();
}

// ── App struct ──

pub struct App {
    pub(crate) window: Option<Arc<Window>>,
    pub(crate) gpu: Option<GpuState>,
    pub(crate) text: Option<TextState>,
    pub(crate) file_path: Option<PathBuf>,
    pub(crate) current_theme: Theme,
    pub(crate) native_menu: Option<NativeMenu>,
    pub(crate) workspace: Workspace,
    pub(crate) ui_shell: UiShell,
    pub(crate) file_history: FileHistory,
    pub(crate) scale_factor: f64,
    pub(crate) running: bool,
    pub(crate) needs_redraw: bool,
    pub(crate) sidebar_animating: bool,
    pub(crate) modifiers: winit::keyboard::ModifiersState,
    pub(crate) mouse: MouseState,
    pub(crate) frame_cache: FrameCache,
    pub(crate) last_scroll_time: Instant,
    pub(crate) reshape_worker: Option<ReshapeWorker>,
    pub(crate) shared_font_system: Option<Arc<Mutex<shaping::FontSystem>>>,
    pub(crate) reshape_generation: u64,
    pub(crate) pending_reshapes: HashSet<usize>,
    pub(crate) skip_reshape_submit: bool,
    pub(crate) last_reshape_anchor: usize,
    pub(crate) last_render_time: std::time::Instant,
    pub(crate) last_rr_time: std::time::Instant,
    pub(crate) pending_resize: Option<winit::dpi::PhysicalSize<u32>>,
    pub(crate) last_resize_handled: Instant,
    pub(crate) last_cursor_visible: bool,
    pub(crate) window_focused: bool,
    pub(crate) event_loop_proxy: Option<EventLoopProxy<AppEvent>>,
    pub(crate) preedit_text: String,
    pub(crate) preedit_cursor: Option<(usize, usize)>,
}

// ── 纯 getter（仅访问字段/Setting，无逻辑）──

impl App {
    pub(crate) fn screen_width(&self) -> f32 {
        self.gpu.as_ref().map(|g| g.ctx.config.width as f32).unwrap_or(800.0)
    }

    pub(crate) fn screen_height(&self) -> f32 {
        self.gpu.as_ref().map(|g| g.ctx.config.height as f32).unwrap_or(600.0)
    }

    pub(crate) fn visible_rows(&self, screen_height: f32) -> usize {
        Settings::with(|s| s.visible_rows(screen_height, self.content_top_offset()))
    }

    pub(crate) fn visible_height_lines(&self, screen_height: f32) -> f64 {
        Settings::with(|s| s.visible_height_lines(screen_height, self.content_top_offset()))
    }

    pub(crate) fn content_top_offset(&self) -> f32 {
        let tbh = self.workspace.current_tab_bar_height();
        if matches!(Settings::with(|s| s.view_mode), ui::view_mode::ViewMode::Sidebar) {
            return ui::title_bar::title_bar_height();
        }
        tbh
    }

    pub(crate) fn viewport_content_width(&self, dv: &DocumentView) -> f32 {
        let screen_w = self.screen_width();
        // 减去边距等
        let margin = Settings::with(|s| s.editor_left_margin());
        screen_w - margin - Settings::with(|s| s.scrollbar_reserve())
    }

    pub(crate) fn current_tab_bar_height(&self) -> f32 {
        self.workspace.current_tab_bar_height()
    }
}

#[cfg(test)]
mod app_getter_tests {
    use super::*;

    #[test]
    fn screen_width_returns_fallback_without_gpu() {
        let app = App::new(None);
        assert_eq!(app.screen_width(), 800.0);
    }

    #[test]
    fn screen_height_returns_fallback_without_gpu() {
        let app = App::new(None);
        assert_eq!(app.screen_height(), 600.0);
    }

    #[test]
    fn viewport_content_width_accounts_for_margins() {
        let app = App::new(None);
        let dv = crate::document_view::DocumentView::new(vec!["test".into()], 40, 600.0);
        let w = app.viewport_content_width(&dv);
        assert!(w > 0.0);
    }
}
```

---

### Task 8: 更新 `lib.rs` 最终版

**Files:**
- Modify: `crates/app/src/lib.rs`

- [ ] **Step 1: 确认 lib.rs 包含所有新模块声明**

```rust
//! edit+ application crate.
//!
//! Provides the winit + wgpu application lifecycle.

pub mod actions;
pub mod app;
pub mod app_dispatch;
pub mod app_init;
pub mod app_lifecycle;
pub mod app_event;
pub mod app_renderer;
pub mod app_reshape;
pub mod app_scroll;
pub mod app_search;
pub mod app_tab;
pub mod app_window;
pub mod cli;
pub mod commands;
pub mod content_hash;
pub mod cursor_motion;
pub mod display_line_map;
pub mod document_view;
pub mod gpu;
pub mod input;
pub mod line_index;
pub mod mouse;
pub mod render_cache;
pub mod render_pipeline;
pub mod render_state;
pub(crate) mod settings_io;
pub mod snap_tree;
pub mod text_rasterize;
pub(crate) mod sys;

pub mod events;
pub mod file_history;
pub mod frame_cache;
pub mod menu_handler;
pub mod measure_adapter;
pub mod native_menu;
pub mod reshape_worker;
pub mod search_state;
pub mod workspace;
pub mod editor_host;
pub mod paint_backend;
pub mod ui_shell;

pub use app::App;
pub use app_event::AppEvent;
pub use gpu::{GpuError, headless_init};
```

---

### Task 9: 编译 & 测试

- [ ] **Step 1: 完整编译**

```bash
cargo build --workspace 2>&1
```

预期：成功，无错误。如有 "unused import" 警告，在对应文件中清理。

- [ ] **Step 2: 运行测试**

```bash
cargo test --workspace 2>&1
```

预期：所有测试通过（包括从 app.rs 移走的 76 个测试现在在各个新文件中运行）。

- [ ] **Step 3: 处理私有方法可见性**

如果编译报 "method not found" 错误，说明某个 `fn`（私有）方法被跨文件调用但未改为 `pub(crate)`。找到该方法声明，加 `pub(crate)`。

已知需要改可见性的方法：
- `move_cursor_visual`: `fn` → `pub(crate) fn`
- `page_up`: `fn` → `pub(crate) fn`
- `page_down`: `fn` → `pub(crate) fn`
- `extend_selection_visual`: `fn` → `pub(crate) fn`
- `config_dir`: `fn` → `pub(crate) fn`
- `load_file`: `fn` → `pub(crate) fn`
- `execute_batch_close`: `fn` → `pub(crate) fn`
- `save_window_geometry`: `fn` → `pub(crate) fn`
- `resize`: `fn` → `pub(crate) fn`
- `handle_sidebar_key_action`: `fn` → `pub(crate) fn`

- [ ] **Step 4: 清理 unused imports**

```bash
cargo clippy --workspace -- -A clippy::all 2>&1 | grep "unused"
```

只关注新文件中的 unused import 警告，逐个清理。

---

### Task 10: Git commit

- [ ] **Step 1: 查看变更**

```bash
git diff --stat
```

预期输出类似：
```
 crates/app/src/app.rs         | ~3000 deletions
 crates/app/src/app_dispatch.rs | +XXX
 crates/app/src/app_reshape.rs  | +XXX
 crates/app/src/app_scroll.rs   | +XXX
 crates/app/src/app_search.rs   | +XXX
 crates/app/src/app_tab.rs      | +XXX
 crates/app/src/app_window.rs   | +XXX
 crates/app/src/lib.rs          | +6
```

- [ ] **Step 2: 提交**

```bash
git add crates/app/src/
git commit -m "refactor: split app.rs (3328→120 lines) into 6 domain files

- app_search.rs: search logic
- app_reshape.rs: reshape pipeline + zoom
- app_scroll.rs: scroll + cursor movement
- app_tab.rs: tab/file/history/workspace integration
- app_window.rs: window init/resize/shell layout/IME
- app_dispatch.rs: command routing (dispatch/execute/handle_command)

App struct + pure getters remain in app.rs (~120 lines).
All tests stay in their respective files per Rust convention."
```

---

## 风险与缓解

| 风险 | 缓解 |
|------|------|
| 私有方法跨文件不可见 | 代码审查时已标记 10 个需要 `fn → pub(crate) fn` 的方法 |
| `use` 语句遗漏导致编译失败 | 每个新文件都列出了所需 imports |
| 重复定义冲突（新文件创建后 app.rs 未删除旧方法） | Task 7 精确列出删除行范围 |
| 测试遗漏 | 每个测试模块指定了目标文件和行范围 |
