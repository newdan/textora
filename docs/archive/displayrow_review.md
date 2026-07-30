# DisplayRow 实施审查报告

> 审查日期：2026-05-31
> 代码状态：306 测试通过，0 失败，0 警告

---

## 1. 需求覆盖对照

对照 `displayrow.md` 的 7 个阶段，逐条检查：

| 阶段 | 要求 | 状态 | 说明 |
|------|------|------|------|
| 1. DisplayRow 类型 | 新类型 + 算术 + From 转换 | ✅ 完成 | `viewport.rs:22-71` |
| 1. Viewport 字段改造 | scroll_y→scroll_top, scroll_line→first_visible_doc_line | ✅ 完成 | `viewport.rs:75-95` |
| 1. 新增 API | first_visible_row, visible_display_range, scroll_to_row | ✅ 完成 | `viewport.rs:118-170` |
| 2. advance_cache 结构变更 | 添加 DisplayRow 字段 | ❌ 未做 | 仍使用 `(usize, usize, Vec<(usize, f32)>)` |
| 2. hit_test 返回 DisplayRow | 返回 HitResult{display_row, ...} | ❌ 未做 | 仍返回 `Option<(usize, usize)>` |
| 3. Autoscroll 重写 | NOT-IN-VLI 用 scroll_to_doc_line | ✅ 完成 | `app.rs:1248-1258` |
| 3. ensure_cursor_visible 移除 | 统一由 autoscroll 处理 | ❌ 未做 | 仍保留两个入口 |
| 4. move_cursor_visual 4b/4c | 用 scroll_to_row 替代直接赋值 | ❌ 未做 | 4b/4c 逻辑未改动 |
| 5. 清理旧接口 | 移除 scroll_line / visible_range | ✅ 完成 | 全部替换 |
| 6. total_visual_lines 修复 | shape_visible_lines 中设置 | ❌ 未做 | `set_total_visual_lines` 从未调用 |
| 7. 测试 | 15 个指定测试 | ⚠️ 部分 | 见下方测试覆盖分析 |

**总结：阶段 1/3/5 完成，阶段 2/4/6 未完成。**

---

## 2. 已发现的 Bug

### Bug 1（中）：`set_total_visual_lines` 从未调用

**位置：** `app.rs` shape_visible_lines 末尾（缺失）

**问题：** `new_vli.total_visual_lines()` 在 shape 末尾可用，但从未写入 `dv.viewport.set_total_visual_lines(...)`。导致 `clamp_scroll_top()` fallback 到 `total_lines`（文档行数），word-wrap 场景下允许过度滚动。

**影响：** 大文件 + 窄窗口（大量 wrap）时，鼠标滚轮可以滚过文档实际末尾，出现空白区域。

**修复方案：** 在 `shape_visible_lines` 的 `self.visual_line_index = new_vli;` 之后加一行：
```rust
dv.viewport.set_total_visual_lines(self.visual_line_index.total_visual_lines());
```

### Bug 2（低）：`resize` 不同步 `first_visible_doc_line`

**位置：** `viewport.rs:112-116`

**问题：** `resize` 调用 `clamp_scroll_top()` 可能改变 `scroll_top`，但不调用 `sync_doc_line_from_scroll_top()`。下一帧 shape 的 `update_first_visible_doc_line()` 会修正，但中间有一帧 `visible_doc_line_range()` 可能返回错误值。

**影响：** resize 瞬间可能出现一帧闪烁（极低概率，用户不可感知）。

**修复方案：** 在 `resize` 末尾加 `self.sync_doc_line_from_scroll_top();`

### Bug 3（中）：`ensure_cursor_visible` 与 autoscroll 双入口

**位置：** `document_view.rs:550-558`, `document_view.rs:799-808`, `app.rs:1220-1265`

**问题：** 两个独立的 autoscroll 逻辑：
1. `ensure_cursor_visible()` — 在 `execute_edit_command` 中调用，用文档行空间
2. `shape_visible_lines` 末尾的 autoscroll — 每帧执行，用 DisplayRow 空间

两者都会修改 `scroll_top`，可能冲突。

**影响：** 正常情况下两者结果一致（文档行 ≈ DisplayRow 在无 wrap 时）。在 wrap 场景下，`ensure_cursor_visible` 的文档行级判断可能不够精确，但下一帧 shape 的 autoscroll 会修正。

**修复方案（Phase 5）：** 移除 `ensure_cursor_visible()`，在 `execute_edit_command` 中不调用它，统一由 shape 末尾的 autoscroll 处理。

### Bug 4（低）：`sync_doc_line_from_scroll_top` 的 clamp 精度

**位置：** `viewport.rs:158-162`

**问题：** clamp 到 `total_lines - 1`。当 word-wrap 导致 `scroll_top` 远大于 `total_lines` 时，`first_visible_doc_line` 被拉到最后一条文档行，`visible_doc_line_range()` 返回很小的范围。

**影响：** 仅在 `total_visual_lines` 已设置且 `scroll_top > total_lines` 时触发。此时 VLI 会修正，但中间有一帧可能不精确。

**测试覆盖：** `sync_clamp_with_wrapping_overestimate` 测试已覆盖此场景并验证 VLI 修正。

---

## 3. 测试覆盖分析

### 已有测试（306 个）

| 模块 | 测试数 | 覆盖 |
|------|--------|------|
| viewport::display_row_tests | 5 | DisplayRow 类型 |
| viewport::viewport_tests | 16 | Viewport 核心 API |
| viewport::update_first_visible_doc_line_tests | 4 | VLI 映射 |
| viewport::autoscroll_displayrow_tests | 4 | autoscroll 单元测试 |
| viewport::visual_line_index_tests | 4 | VLI 数据结构 |
| app::tests (autoscroll) | 3 | 集成 autoscroll 测试 |
| app::tests (其他) | ~30 | 原有测试 |
| document_view::tests | ~38 | 原有测试 |
| document_view::cursor_visual_tests | ~24 | 原有测试 |
| 其他 | ~178 | input/selection/boundary 等 |

### 缺失的测试

| 测试名 | 覆盖场景 | 优先级 |
|--------|---------|--------|
| `set_total_visual_lines_in_shape` | shape 设置 total_visual_lines | P0 |
| `resize_syncs_first_visible_doc_line` | resize 后 visible_doc_line_range 正确 | P1 |
| `ensure_cursor_visible_uses_scroll_to_doc_line` | ensure_cursor_visible 不直接赋值 | P1 |
| `autoscroll_wrap_line_cursor_follows` | 长 wrap 行 autoscroll 精确性 | P1 |
| `scroll_top_overshoot_with_wrap` | total_visual_lines > total_lines 时 clamp | P2 |

---

## 4. 性能缺陷

### 4.1 Word Wrap 每帧重算（已有问题，未恶化）

`shape_visible_lines` 每帧对所有可见行重新做 word wrap。本次改动未引入新的性能问题，也未优化此路径。

**建议：** 后续引入 WrapMap 增量更新（Zed 的方案）。

### 4.2 VLI 每帧重建（已有问题，未恶化）

`new_vli` 在 shape 循环中从零构建。本次改动未引入新的性能问题。

### 4.3 `sync_doc_line_from_scroll_top` 的开销（可忽略）

`scroll_by` 和 `scroll_to_row` 每次调用都执行一次 `floor()` + `min()`，O(1) 开销，可忽略。

### 4.4 `advance_cache` 的 heap 分配（已有问题，未恶化）

每帧 `clear()` + `push()` 导致频繁 alloc/dealloc。本次改动未优化此路径。

---

## 5. 回归验证：已有分析文档的 bug 修复状态

| 已知 Bug | 来源文档 | 修复状态 | 说明 |
|---------|---------|---------|------|
| NOT-IN-VLI autoscroll 赋值 scroll_line 是 no-op | 滚动两类异常根因分析.md | ✅ 已修复 | 改用 scroll_to_doc_line() |
| 3d else 分支 scroll_to 死循环 | 滚动两类异常根因分析.md | ✅ 已修复 | 旧 3d 逻辑已被替换 |
| visible_range 不补偿 scroll_visual_offset | 滚动两类异常根因分析.md | ✅ 已修复 | scroll_visual_offset 已移除，由 scroll_top 小数部分替代 |
| cursor_visual_line 在 viewport 外不更新 | 滚动两类异常根因分析.md | ⚠️ 部分 | cursor_visual_line 仍用 usize::MAX 哨兵，但 autoscroll 已修正 |
| scroll_visual_step_down 跨行后 count 过期 | plans_viewport_offset_revision.md A1 | ✅ 已修复 | scroll_visual_step_down 已移除 |
| move_cursor_visual 4b 不移动光标 | plans_viewport_offset_revision.md A2 | ❌ 未修复 | 4b 逻辑未改动 |
| move_cursor_visual 4c 不使用 sticky_x | plans_viewport_offset_revision.md A3 | ❌ 未修复 | 4c 逻辑未改动 |
| total_visual_lines 从未设置 | plans_viewport_offset_revision.md C1 | ❌ 未修复 | Phase 6 未完成 |

---

## 6. 待修复项（按优先级）

### P0：必须修

1. **设置 total_visual_lines** — 在 shape_visible_lines 末尾加一行
2. **resize 同步 first_visible_doc_line** — 在 resize 末尾加一行

### P1：应该修

3. **移除 ensure_cursor_visible 双入口** — 统一由 autoscroll 处理
4. **advance_cache 添加 DisplayRow** — 为后续 hit_test 改造做准备

### P2：可以后续做

5. **move_cursor_visual 4b/4c 改造** — 需要 first/last_line_visual_lines 字段
6. **hit_test 返回 DisplayRow** — 依赖 advance_cache 结构变更
7. **Word wrap 增量更新** — 大规模重构

---

## 7. 结论

**已完成的核心目标：**
- ✅ DisplayRow 类型引入，统一坐标空间的基础已建立
- ✅ Viewport 字段重命名（scroll_y→scroll_top, scroll_line→first_visible_doc_line）
- ✅ NOT-IN-VLI autoscroll bug 修复（根因修复）
- ✅ 旧接口清理完毕，零警告

**遗留问题：**
- ❌ advance_cache / hit_test / move_cursor_visual 未改造
- ❌ total_visual_lines 未设置（Phase 6）
- ❌ ensure_cursor_visible 双入口未合并（Phase 5）
- ❌ 4b/4c 的 sticky_x 精确性未改善

**总体评估：** 核心 bug（自动滚动）已修复，坐标空间基础已建立。
剩余工作属于 Phase 2/4/5/6 的深化改造，不影响基本功能。
