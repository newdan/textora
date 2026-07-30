# 大文件拆分设计（app.rs + document_view/mod.rs + sidebar.rs）

> 已确认 · 2026-06-14

## 一、app.rs（3328行 → ~120行）

同 `2026-06-14-app-rs-split-design.md`，不再重复。

拆分出 6 个文件：`app_search.rs`, `app_reshape.rs`, `app_scroll.rs`, `app_tab.rs`, `app_window.rs`, `app_dispatch.rs`。

---

## 二、document_view/mod.rs（1275行 → ~600行 + 3新文件）

### 现状

已有子模块：`cursor.rs`（光标状态）、`display.rs`（显示映射）

mod.rs 中 `DocumentView` 的 80+ 个方法分散在多个职责域。

### 拆分方案：保守 3 新文件

```
document_view/
├── mod.rs           (~600行) — 构造/持久化 + 光标移动 + 基础查询 + 搜索/高亮
├── cursor.rs        (已有)
├── display.rs       (已有)
├── selection.rs     (~280行) — 选择操作(10个extend_*) + 剪贴板(copy/cut/paste) + 
│                                word_select_at, has_selection, selection_range,
│                                clear_selection, select_all, delete_selection,
│                                extract_selected_text, count_selection_chars,
│                                paste_text, ensure_selection_active
├── edit.rs          (~200行) — insert_at_cursor, delete_backward/forward,
│                                undo/redo, sync_after_edit_*, sync_cursor*,
│                                set_cursor_offset_synced, assert_cursor_synced
├── visible.rs       (~180行) — visible_line, visible_line_wrap, visible_lines,
│                                visible_line_count*, visible_line_key*,
│                                visible_doc_range
└── test_*.rs        (已有 7 个文件)
```

### 保留在 mod.rs 的方法

| 组 | 方法 |
|----|------|
| 构造/持久化 | `new`, `from_file`, `save`, `save_as` |
| 基础查询 | `tb`, `line_count`, `is_empty`, `buffer_len`, `line_byte_offset`, `line_byte_length`, `doc_line_bytes`, `resize`, `set_crlf` |
| 光标查询 | `cursor`, `cursor_mut`, `cursor_line`, `cursor_line_cached`, `cursor_column` |
| 光标移动（10个） | `cursor_move_left/right/up/down/word_left/word_right/to_line_start/to_line_end/to_offset`, `indent_column_offset` |
| 翻页/视觉 | `page_up`, `page_down`, `move_cursor_visual`, `ensure_cursor_visible` |
| 搜索/高亮 | `perform_search`, `highlights_for_line`, `invalidate_highlights_from` |
| 其他 | `rebuild_viewport` |
| 自由函数 | `replace_null_bytes`, `normalize_paste_text` |

### 可见性

- 移到子模块的方法：当前 `pub`/`pub(crate)` 不变
- `fn visible_doc_range` → `pub(crate) fn`（被 visible.rs 跨文件调用）

---

## 三、ui/src/sidebar.rs（1647行 → sidebar/ 目录）

### 现状

单文件包含：类型定义（5 struct, 4 enum）、状态机（SidebarState 3 个 impl 块）、edge drag、hover 逻辑、布局、测试（~645行）。

### 拆分方案：参考 tab_bar/ 目录模式

```
ui/src/sidebar/            （新建目录，替换单个 sidebar.rs 文件）
├── mod.rs        (30行)  — 模块声明 + 公开 re-exports（pub use self::*）
├── types.rs      (90行)  — SidebarConfig, Visibility, SidebarInput,
│                           SidebarKey, SidebarAction, SidebarHoverButton
├── layout.rs     (80行)  — SidebarLayoutItem, SidebarLayout
├── drag.rs       (150行) — EdgeDragState + 9 个 drag 相关测试
├── persistent.rs (120行) — SidebarPersistent struct + impl
└── state.rs      (~1085行)— SidebarState + 3 impl 块 + ~33 个测试
```

### 各文件内容

**`types.rs`（~110行）：**
- `SidebarConfig` struct + impl（`new_default`, `clamp_width`）
- `Visibility` enum + Default impl
- `SidebarInput<'a>` struct
- `SidebarKey` enum
- `SidebarAction` enum
- `SidebarHoverButton` enum
- 测试：`clamp_width_below_min`, `clamp_width_above_max`, `clamp_width_within_range_unchanged`（~20行）

**`layout.rs`（~80行）：**
- `SidebarLayoutItem` struct
- `SidebarLayout` struct

**`drag.rs`（~150行）：**
- `EdgeDragState` struct + impl
- 测试：`sidebar_width_drag_*` (4个), `sidebar_drag_*` (5个)（~90行）

**`persistent.rs`（~120行）：**
- `SidebarPersistent` struct + impl（`new`, `current_width`, `editor_left_offset`, `tick`, `on_mouse_move`）

**`state.rs`（~1085行）：**
- `SidebarState` struct + 3 个 impl 块
- 核心函数：`update_layout`, `on_mouse_move`, `tick`, `on_key`, `paint`, `hit_test_px`, `build_settings_menu`, `dispatch_menu_click`, `open_settings_menu`, `on_drag_start`, `on_drag`, `on_drag_end`
- 测试：其余 ~33 个测试（hover、key、click、scroll、settings menu、mode switch 等，~535行）

### 引用路径更新

所有 `use ui::sidebar::*` 引用保持不变（Rust 自动解析目录中的 mod.rs）。

唯一需要在 `lib.rs` 中改的：`pub mod sidebar;` 保持不变（自动找 `sidebar/mod.rs`）。

---

## 不做

- 不改变任何函数签名
- 不改变测试逻辑
- 不修改 `crates/ui/src/widgets/sidebar.rs`（Widget 实现层）
