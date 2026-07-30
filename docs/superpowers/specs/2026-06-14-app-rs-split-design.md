# app.rs 拆分设计

> 状态：已确认 · 2026-06-14

## 背景

`crates/app/src/app.rs` 3328 行（~2260 生产 + ~1068 测试），涵盖 12+ 个职责域。现有 `app_lifecycle.rs` 和 `app_renderer.rs` 已示范跨文件 `impl App` 拆分的可行模式。

## 目标

- app.rs 降为 ~120 行薄壳，只放 struct 定义 + 零逻辑 getter
- 6 个新文件，每个职责单一，独立可测
- 不改变任何行为，不破坏公共 API
- 测试留在各拆分文件底部（Rust 惯例）

## 文件分配

### `app.rs`（~120 行）— 薄壳

- `App` struct 定义（所有 `pub(crate)` 字段原样保留）
- 自由函数：`compute_cursor_phase`, `reset_after_edit`
- 纯 getter：
  - `screen_width`, `screen_height`
  - `visible_rows`, `visible_height_lines`
  - `viewport_content_width`, `content_top_offset`
  - `current_tab_bar_height`

```rust
// app.rs 不再包含任何有逻辑的方法。所有 impl App 块移入子模块。
pub struct App { /* 所有字段不变 */ }
```

### `app_tab.rs`（~350 行）— Tab + 文件 + 历史

打开文件 → 建 Tab → 记历史是一个紧密链路，不拆散：

```
open_file, open_file_dialog, load_file
new_empty_tab
try_close_tab_with_prompt, try_close_multiple_with_prompt, execute_batch_close
update_tab_layout
handle_workspace_effect        ← 15 处调用，核心联动方法
update_document_edited, update_window_title
save_workspace_snapshot
record_tab_to_history, record_all_tabs_to_history, save_history
config_dir (static)
```

### `app_dispatch.rs`（~300 行）— 命令路由

顶层路由，调用各子模块串联端到端行为：

```
dispatch                  ← 所有 AppAction 分发
dispatch_menu_action
execute_commands          ← 所有 AppCommand 分发
handle_command            ← 编辑命令，~385 行
handle_sidebar_key_action
```

### `app_scroll.rs`（~180 行）— 滚动 + 光标

```
handle_scroll
move_cursor_visual, page_up, page_down
extend_selection_visual
```

### `app_search.rs`（~80 行）— 搜索

```
perform_search_for_active_doc
```

完全独立，只访问 `workspace.doc_views`。

### `app_reshape.rs`（~200 行）— Reshape 管线 + 缩放

```
invalidate_reshape
drain_reshape_results
submit_reshape_ahead
post_shape_update
apply_zoom
```

### `app_window.rs`（~150 行）— 窗口 + 生命周期

```
init_window
resize, flush_pending_resize, handle_resize
save_window_geometry
quit_app
update_ime_cursor_area
has_active_animation, compute_next_wake_time
```

## 可见性

| 当前 | 变更 | 原因 |
|------|------|------|
| `pub(crate) fn` | 不变 | 跨文件 impl 块可在 crate 内相互访问 |
| `fn` (private) | 同文件内 private 不变；跨文件需要时升为 `pub(crate)` | 大部分 private 方法跟对应 impl 块在同一文件 |

## lib.rs 追加

```rust
// 已在 lib.rs 声明但内容从 app.rs 迁移
pub mod app_tab;
pub mod app_dispatch;
pub mod app_scroll;
pub mod app_search;
pub mod app_reshape;
pub mod app_window;
```

## 测试

所有单元测试留在对应文件的 `#[cfg(test)] mod tests {}` 中：

| 文件 | 测试主题 |
|------|----------|
| `app.rs` | getter 测试、zoom 测试（sim_zoom_*、apply_zoom_*） |
| `app_tab.rs` | tab 操作、文件操作、历史、窗口标题 |
| `app_scroll.rs` | 滚动、光标移动、翻页 |
| `app_reshape.rs` | reshape 管线、invalidate |
| `app_window.rs` | ui_shell、窗口几何 |

## 实施顺序

依赖关系：搜索最独立 ← reshape ← 滚动/tab ← 路由层。

1. `app_search.rs`（无内部依赖）
2. `app_reshape.rs`（仅访问字段）
3. `app_scroll.rs`（访问 workspace）
4. `app_tab.rs`（依赖 reshape 的 `invalidate_reshape`）
5. `app_window.rs`（依赖 reshape）
6. `app_dispatch.rs`（调用以上所有）
7. 清理 `app.rs`，更新 `lib.rs`
8. `cargo build --workspace && cargo test --workspace`

## 不做

- 不改变任何方法签名
- 不改变测试逻辑
- 不拆 `commands.rs`、`document_view/mod.rs`、`sidebar.rs`（后续独立评估）
