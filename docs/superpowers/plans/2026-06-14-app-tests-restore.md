# 恢复并修复 `app_tests.rs` 执行方案

## Context

`app_tests.rs`（1159行，44个测试函数）在提交 `13391ff0` 中从 `app.rs` 拆分出来，但因 `lib.rs` 缺少 `mod app_tests;` 声明，从未被编译执行。该文件在后续多个提交中被修改（新增测试、修复），最终在 HEAD (`c7dbafb0`) 作为"孤儿文件"删除，提交信息注明"后续单独恢复修复"。

自测试编写以来，代码经历了大量重构：
- `sticky_x`/`cursor_visual_line`/`last_cursor_offset` 从 `App` 移至 `CursorRenderState`（挂在 `DocumentView` 下）
- `display`（含 `display_map`、`viewport`）从 `App` 移至 `DocumentView.display`
- `compute_visual_lines` 从 `crate::render_pipeline` 移至 `ui::layout`
- `Settings` 从 `App` 字段变为全局 `Settings::with(|s| ...)` 访问模式
- `status_bar_text()` 方法被移除，功能迁移至 `ui_shell` + `StatusBarInput::build_text()`
- `FrameCache` 新增 `cluster_pool` 字段

## 恢复步骤

### Step 1: 从 git 恢复文件

```bash
git show 594a4889:crates/app/src/app_tests.rs > crates/app/src/app_tests.rs
```

`594a4889` 是删除前最后一个修改该文件的提交。

### Step 2: 接入模块树

在 `crates/app/src/app.rs` 末尾添加：

```rust
#[cfg(test)]
#[path = "app_tests.rs"]
mod app_tests;
```

**选择此方案的理由：**
- 文件中的 `use super::*;` 将其定位为 `app` 模块的子模块，可访问 `App` 的私有方法（`move_cursor_visual`、`extend_selection_visual` 等）
- 无需修改任何现有 visibility（避免将私有方法改为 `pub(crate)`）
- `#[path]` 属性允许文件保持在 `crates/app/src/app_tests.rs` 原路径
- 父模块中已有的 `use` 导入（如 `AdvanceCacheEntry`、`compute_selection_highlight_quads`）通过 `use super::*;` 自动对测试可见

## 修复清单

### A. 导入路径变更（1处）

| 行 | 旧 | 新 |
|----|----|-----|
| 3 | `use crate::render_pipeline::compute_visual_lines;` | `use ui::layout::compute_visual_lines;` |

### B. 字段搬迁：`app.display` → `dv.display`（~25处）

`display` 字段已从 `App` 移至 `DocumentView`。所有 `app.display.display_map.*` 和 `app.display.*` 需改为通过 `dv` 访问。

影响范围（按函数）：
- `app_with_content` (L90): `app.display.display_map.set_entries(...)` → `dv.display.display_map.set_entries(...)`
- `move_up_into_skipped_area_moves_cursor_byte` (L214-215, 222-224)
- `move_down_past_visible_preserves_sticky_x` (L244-245, 249, 253-255)
- `move_down_wrapped_line_uses_correct_byte_offset` (L273-275, 279)
- `move_down_wrapped_line_boundary_no_stall` (L297-300, 304, 308)
- `autoscroll_in_vli_no_scroll` (L709-710, 723-726)
- `advance_cache_clear_does_not_invalidate_wrap_index` (L748, 761, 768)
- `first_visible_dr_captured_with_scroll_offset` (L782, 786-788, 793-797)
- `regression_wrap_shift_down_extends_by_visual_line` (L887-888)
- `close_tab_*` 系列 (L1012-1015, 1036-1037, 1060-1062)

### C. 字段搬迁：`app.sticky_x` → `dv.cursor_render_state.sticky_x`（5处）

L225, L256, L282, L310, L907

### D. 字段搬迁：`app.cursor_visual_line` → `dv.cursor_render_state.cursor_visual_line`（6处）

L226, L257, L283, L311, L327（赋值 `app.cursor_visual_line = None` → `dv.cursor_render_state.cursor_visual_line = None`）, L906

### E. 字段搬迁：`app.last_cursor_offset` → `dv.cursor_render_state.last_cursor_offset`（1处）

L735

### F. 字段搬迁：`app.settings.line_height` → `Settings::with(\|s\| s.line_height)`（1处）

L329

### G. 方法搬迁：`app.close_tab(idx)` → `app.workspace.close_tab(idx)`（4处）

L1007, L1032, L1052, L1057。`close_tab` 始终在 `Workspace` 上，从未在 `App` 上存在过。

### H. 删除无效测试：`status_bar_text` 测试（3个测试函数）

L612-669 的 3 个测试（`status_bar_caches_selection_counts`、`status_bar_cache_invalidated_on_selection_change`、`status_bar_cache_cleared_when_no_selection`）调用已经不存在的 `app.status_bar_text()` 方法。

该功能已迁移至 `ui::widgets::status_bar::build_text(StatusBarInput, &mut StatusBarCache) -> String`，且 `app_renderer.rs:191` 中构造 `StatusBarInput` 的逻辑更为简单（无缓存层）。这三个测试直接删除。

### I. 修复重复 `#[test]` 属性（1处）

L924-928：`regression_delete_selection_cursor_line_cache` 函数上有 3 个 `#[test]` 属性，应是复制粘贴错误。保留 1 个。

### J. `dv.tb` 访问方式确认（无需修改）

L1157 的 `dv.tb.to_string()` — `tb` 在 `DocumentView` 上仍是 `pub(crate)` 字段，直接访问有效，无需修改。

## 修改文件清单

| 文件 | 操作 |
|------|------|
| `crates/app/src/app_tests.rs` | 从 git 恢复 → 修改 ~60 行 |
| `crates/app/src/app.rs` | 添加 2 行（`#[cfg(test)]` + `#[path]` + `mod app_tests;`） |

## 验证

```bash
# 仅编译测试（不运行），快速检查编译错误
cargo test -p app -- app_tests --no-run

# 编译通过后运行全部 41 个测试（44 - 3 个已删除的 status_bar 测试）
cargo test -p app -- app_tests

# 确认没有破坏现有测试
cargo test -p app
```

## 风险与缓解

- **`app_with_content` 辅助函数可能需要调整**：当前 `App::new(None)` 签名仍为 `pub fn new(file_path: Option<PathBuf>)`，匹配。但若构造函数内部逻辑变化导致辅助函数无法正确初始化，需根据编译错误调整。
- **`FrameCache` 新增 `cluster_pool` 字段**：测试通过字段赋值（如 `app.frame_cache.first_line.visual_lines = ...`）而非结构体字面量构造，不受影响。
- **`LineCache` 结构保持兼容**：`visual_lines: Vec<(usize, usize, f32)>`、`clusters: Vec<(usize, usize, f32)>`、`doc_offset: usize` 三个字段均未改变类型。
