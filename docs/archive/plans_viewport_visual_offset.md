# Viewport 虚拟行偏移支持 — 改动分析

> 注：CLAUDE.md 约定实施计划应写成 `plans.md`。本文件名带主题后缀以避免覆盖已有计划，最终落盘前需要确认是否改名为 `plans.md` 或合并进去。

## 问题根因

当前 `Viewport` 只跟踪 `scroll_line`（文档行号），`visible_range()` 返回的是文档行范围。

当 word wrap 把一条文档行拆成多条虚拟行时，viewport 只能按文档行粒度滚动。如果光标在同一文档行内但位于屏幕外的虚拟行上，viewport 无法把那条虚拟行滚进屏幕。

**具体复现路径：**
1. 打开一个有超长行的文件，word wrap 开启
2. 超长行被拆成 N 条虚拟行（N > visible_rows）
3. 用下箭头移动光标到该行的第 visible_rows+1 条虚拟行
4. `shape_visible_lines()` 末尾的滚动逻辑发现 `cursor_doc_line` 仍在 viewport 内（`range.start <= cursor_doc_line < range.end`），不触发滚动
5. 光标视觉行 >= visible_rows，触发 `scroll_to(cursor_doc_line - visible_rows + 1)`，但这把整行滚出屏幕，而不是滚动虚拟行偏移

## 现状快照

| 文件 | 关键结构 | 当前行为 |
|------|---------|---------|
| `viewport.rs:8-17` | `scroll_line: usize` / `visible_rows: usize` / `total_lines: usize` / `total_visual_lines: Option<usize>` | 只有文档行偏移，无"首行内部从第几条虚拟行开始"的概念。`total_visual_lines` 只用于估算总虚拟行数（滚动条/`is_at_visual_bottom`），不参与渲染 |
| `viewport.rs:61-65` | `visible_range()` | 返回 `scroll_line..(scroll_line + visible_rows).min(total_lines)` |
| `viewport.rs:37-58` | `resize` / `scroll_to` / `scroll_up` / `scroll_down` | 均按文档行操作，仅 `clamp()` 维护 `scroll_line` 上界 |
| `app.rs:429` | `shape_visible_lines()` | 遍历 `dv.visible_line_count()`，word wrap 时生成 `visual_lines` 但不跳过偏移 |
| `app.rs:679-697` | 视口滚动逻辑（位于 `shape_visible_lines` 末尾） | 只判断 `cursor_doc_line` 是否在 range 内，无法处理同 doc_line 内虚拟行超出屏幕 |
| `app.rs:273` | `hit_test()` | `vis_line = (py / line_height) as usize`，直接索引 `advance_cache` |
| `app.rs:308` | `move_cursor_visual()` | 用 `cursor_visual_line` 作为 advance_cache 索引，超出范围时跳到上/下文档行 |
| `app.rs:807` | `handle_scroll()` | 鼠标滚轮调 `dv.scroll_down/up(N)`，按文档行滚 |

`shape_cache` 只缓存 `shaper.shape(line_str)` 的结果（裸 cluster 数组），word wrap 每帧基于 `viewport_width` 重算，因此本计划不涉及 shape_cache。

---

## 关键设计决策

### 决策 1：`scroll_visual_offset` 的语义

新增字段 `Viewport::scroll_visual_offset: usize`，表示 `scroll_line` 所指文档行从第几条虚拟行开始显示。

- `scroll_visual_offset == 0` 时行为完全等同当前实现
- `scroll_visual_offset > 0` 时，首条文档行的前 N 条虚拟行**不渲染、不进 advance_cache**（不是渲染后裁剪）—— 因此屏幕上不存在"被部分遮挡"的视觉区域，hit_test/click 不需要特殊处理首行

### 决策 2：`cursor_visual_line` 的语义

`cursor_visual_line` 是 advance_cache 中的相对索引（0 = 屏幕第一条可见虚拟行）。

特殊取值：
- `usize::MAX` —— 哨兵值，表示光标当前不在 advance_cache 中（已在 `app.rs:509` 使用）。当 `cursor_doc_line == scroll_line` 且光标所在虚拟行索引 `< scroll_visual_offset` 时（被跳过区域），同样使用此哨兵。这样阶段 3 的滚动修正逻辑能统一识别"光标不可见"。

### 决策 3：滚动 API 拆成两层

为了解决"按文档行滚动需要 reset offset，按虚拟行滚动需要保留"的冲突：

- `scroll_to(doc_line)` / `scroll_up(delta)` / `scroll_down(delta)` —— 文档行级 API，**重置** `scroll_visual_offset = 0`
- 新增 `scroll_visual_line(doc_line, visual_offset)` —— 虚拟行级 API，精确设置两个字段
- 新增 `scroll_visual_step_down() / scroll_visual_step_up(first_line_visual_count)` —— 单步虚拟行滚动；跨文档行边界时由该方法内部决定是否 reset。调用方需提供首行虚拟行数（来自上一帧的 `scroll_line_visual_count`，详见决策 5）

### 决策 4：`Viewport::resize` 不变签名

`resize(visible_rows)` 维持现签名。`scroll_visual_offset` 的 clamp 不在 resize 中处理，因为：
- max offset 依赖该 doc_line 的虚拟行数，需要 word wrap 结果，而 wrap 在 `shape_visible_lines` 中算
- resize 后下一帧 `shape_visible_lines` 自然会触发阶段 3 的修正逻辑，把超界 offset 拉回

resize 时只做一件额外的事：清空缓存的 `scroll_line_visual_count`（设为 0），避免阶段 6 用过期值。

### 决策 5：`scroll_line_visual_count` 的时序

`Viewport::scroll_line_visual_count: usize` 缓存"当前 scroll_line 的虚拟行总数"，由 `shape_visible_lines` 在每帧首行处理时写入，由 `handle_scroll` 读取。

时序约束：
- 文件加载完到首帧 `shape_visible_lines` 之前，字段为 0 —— `handle_scroll` 在读到 0 时 fallback 到 `dv.scroll_down(1)`（按文档行滚），保持当前行为
- resize 后同上 —— resize 中清零，下一帧重算

---

## 改动方案

### 阶段 1：Viewport 增加虚拟行字段与 API

**文件：** `crates/app/src/viewport.rs`

**改动点：**
1. 新增字段：
   - `pub scroll_visual_offset: usize`
   - `pub scroll_line_visual_count: usize`（首行虚拟行数缓存，0 = 未知/未计算）
2. `new()` 中两个字段都初始化为 0
3. **修改现有方法**（不是新增）：
   - `scroll_to / scroll_up / scroll_down`：在内部调 `clamp()` 之后，重置 `scroll_visual_offset = 0`、`scroll_line_visual_count = 0`
   - `resize`：现签名不变，body 中重置 `scroll_line_visual_count = 0`（offset 留给下一帧 shape 修正）
   - `set_total_lines`：重置两个新字段
4. 新增方法：
   - `pub fn scroll_visual_line(&mut self, doc_line: usize, visual_offset: usize)` — 同时设置 `scroll_line` 和 `scroll_visual_offset`，调 `clamp()`，**不**重置 `scroll_line_visual_count`（调用方负责更新或留给下一帧）
   - `pub fn set_scroll_line_visual_count(&mut self, count: usize)` — `shape_visible_lines` 调用
5. `visible_range()` 行为不变（仍返回文档行范围）

**单元测试：**
- `scroll_visual_offset_default_zero`
- `scroll_visual_line_sets_both_fields`
- `scroll_to_resets_visual_offset`
- `scroll_up_down_resets_visual_offset`
- `resize_resets_visual_count_only`（offset 保留，count 清零）
- `set_total_lines_resets_visual_fields`

---

### 阶段 2：`shape_visible_lines()` 渲染时跳过虚拟行偏移

**文件：** `crates/app/src/app.rs:429`

**改动点：**

**2a. 渲染首行时跳过前 N 条虚拟行**

在 `for i in 0..vis_count` 循环中，对 `i == 0` 的迭代：
```text
let skip = dv.viewport.scroll_visual_offset;
// 在生成 visual_lines 之后，从 visual_lines.iter().skip(skip) 开始
// 用于 advance_cache、render、cursor 定位的所有遍历都从 skip 开始
```

如果 `skip >= visual_lines.len()`（offset 越界），整行不渲染，下一帧滚动逻辑会修正。

**2b. 写入 `scroll_line_visual_count`**

`i == 0` 处理完 word wrap 后：
```text
dv.viewport.set_scroll_line_visual_count(visual_lines.len());
```

**2c. 更新 `cursor_visual_line` 计算（app.rs:504-545 块）**

当前逻辑用 `cursor_doc_line - range.start` 作为 `i` 比较。改动：
- 计算 `cursor_vl_in_doc`：光标所在文档行内的虚拟行索引（遍历 `visual_lines` 时按 byte_range 找）
- 若 `cursor_doc_line == scroll_line` 且 `cursor_vl_in_doc < skip`：`cursor_visual_line = usize::MAX`（被跳过区域，光标"不可见"）
- 否则：`cursor_visual_line = visual_line_counter + (cursor_vl_in_doc - skip)` （首行情况；非首行 skip = 0）

**2d. advance_cache 写入对齐 skip**

`for &(vl_start, vl_end, _) in &visual_lines.iter().skip(skip)` —— 跳过的虚拟行不进 cache。这样 `advance_cache[0]` 永远对应屏幕 y=0 的那一行。

**边界：**
- `skip >= visual_lines.len()` —— `scroll_line_visual_count` 仍写入（用于下一次滚轮判断），advance_cache 该行无贡献
- 首行就是空行（`length == 0`，已 continue）—— 不影响

---

### 阶段 3：视口滚动修正逻辑支持虚拟行偏移

**文件：** `crates/app/src/app.rs:679-697`

**改动点：** 替换当前三段 if-else。

**辅助值：**
- `cursor_doc_line = dv.cursor_line()`
- `range = dv.viewport.visible_range()`
- `visible_rows = dv.viewport.visible_rows`
- `cursor_vl_in_doc`：阶段 2c 已计算，需要从 shape 阶段把它传出来（用 self 字段缓存或返回值）
- `cursor_doc_line_visual_count`：光标所在 doc_line 的虚拟行总数，同样从 shape 阶段传出

**修正分支：**

3a. **`cursor_doc_line < range.start`** → `dv.viewport.scroll_to(cursor_doc_line)`（按现行行为，offset reset 为 0）

3b. **`cursor_doc_line >= range.end`** → 与 3a 对称，但需要考虑光标行是超长行的情况：
- 若 `cursor_doc_line_visual_count <= visible_rows` → `scroll_to(cursor_doc_line - visible_rows + 1)`
- 若 `cursor_doc_line_visual_count > visible_rows` → `scroll_visual_line(cursor_doc_line, cursor_vl_in_doc.saturating_sub(visible_rows - 1))`

3c. **`cursor_doc_line == range.start` 且 `cursor_vl_in_doc < scroll_visual_offset`**（被首行 skip 区遮挡）
→ `dv.viewport.scroll_visual_offset = cursor_vl_in_doc;`（直接修改字段，scroll_line 不变）

3d. **光标在 viewport 内但虚拟行超出屏幕**（`cursor_visual_line != usize::MAX && cursor_visual_line >= visible_rows`）
→ 计算光标在屏幕上需要的"虚拟行级"绝对位置：
- 若 `cursor_doc_line_visual_count <= visible_rows`（光标行整行能放下）→ `scroll_to(cursor_doc_line - visible_rows + 1)` 或调整使整行末端贴底
- 若 `cursor_doc_line_visual_count > visible_rows`（超长行）→ `scroll_visual_line(cursor_doc_line, cursor_vl_in_doc - visible_rows + 1)`

3e. **跳转场景**（搜索/Ctrl+G/go-to-line 调用 `scroll_to` 后）
→ `scroll_to` 已 reset offset = 0；下一帧 shape 跑完后，3c/3d 自动把 offset 调到光标可见位置 —— 无需额外代码，但需测试覆盖。

**实现说明：** 当前 `cursor_visual_line` 字段是 `Self` 上的 usize，可以再加两个字段 `cursor_vl_in_doc: usize` 和 `cursor_doc_line_visual_count: usize`，由 shape 阶段写入，由滚动修正块读取。

---

### 阶段 4：`move_cursor_visual()` 适配虚拟行偏移

**文件：** `crates/app/src/app.rs:308`

`move_cursor_visual` 的语义是**计算目标 byte offset 并移动光标**，视口滚动交给下一帧的阶段 3。但当 `target_vis` 超出 advance_cache 范围时，需要更精确地决定光标落点。

**4a. `target_vis` 在 `[0, advance_cache.len())` 内** —— 现行逻辑不变。

**4b. `target_vis < 0`（向上超出）**

当前实现：跳到上一文档行末尾。这在 word wrap 下不正确 —— 光标应该跳到当前 doc_line 的上一条虚拟行（如果存在），而不是上一文档行。

新逻辑：
- 取 `(first_doc_line, _) = self.advance_cache[0]`
- 若 `first_doc_line == dv.viewport.scroll_line` 且 `dv.viewport.scroll_visual_offset > 0`：
  - 目标虚拟行 = scroll_visual_offset - 1
  - 需要拿到 `first_doc_line` 的完整 word wrap 结果（visual_lines），找到第 `scroll_visual_offset - 1` 条虚拟行的 `(vl_start, vl_end)` 和 cluster advances
  - 用 `sticky_x` 在该条虚拟行的 cluster advances 中找最近列，得到 byte offset
  - `dv.cursor_move_to_offset(line_start + best_offset)`
  - 阶段 3c 会在下一帧调小 `scroll_visual_offset`
- 否则（`first_doc_line` 之上还有文档行）：保持当前"跳到上一文档行末尾"逻辑（但应跳到上一文档行的**最后一条虚拟行**对应位置 + sticky_x，而不是行尾。这是当前 bug 的另一面，但范围超出本计划，记入跟踪）

**实现细节：** 上一条虚拟行的 cluster advances 不在 `advance_cache` 中（被 skip 掉了），需要在 4b 内部重新对 `first_doc_line` 跑一次 word wrap，或者把首行的完整 visual_lines 也缓存起来。建议加字段 `first_line_visual_lines: Vec<...>`，由 shape 阶段写入。

**4c. `target_vis >= advance_cache.len()`（向下超出）**

当前实现：移到下一文档行开头。同样不准。

新逻辑：
- 取 `(last_doc_line, last_clusters) = advance_cache.last()`
- 计算 `last_doc_line` 在屏幕上展示了几条虚拟行（last_doc_line_visual_shown）
- 若 `last_doc_line` 还有更多虚拟行（`last_doc_line_visual_count > scroll_visual_offset_for_last + last_doc_line_visual_shown`）：
  - 目标 = 下一条虚拟行
  - 同样需要该 doc_line 的完整 visual_lines；建议加字段 `last_line_visual_lines: Vec<...>`
  - 用 sticky_x 选列 → 移光标
  - 阶段 3d 下一帧调整 offset
- 否则：保持当前"跳到下一文档行开头"逻辑

**字段汇总（self 上需要新增的渲染→输入数据通道）：**
- `cursor_vl_in_doc: usize`
- `cursor_doc_line_visual_count: usize`
- `first_line_visual_lines: Vec<(usize, usize, f32)>`（首行 word wrap 结果，给 4b 用）
- `last_line_visual_lines: Vec<(usize, usize, f32)>`（末行 word wrap 结果，给 4c 用）

---

### 阶段 5：hit_test 维持现状

**文件：** `crates/app/src/app.rs:273`

阶段 2d 保证了 `advance_cache[0]` 对应屏幕 y=0，所以 `vis_line = py / line_height` 仍直接对应 `advance_cache[vis_line]`，无需改动。

唯一边界：`vis_line >= advance_cache.len()` 已有 `return None`（line 275），覆盖了点击空区域的情况。

**结论：阶段 5 无代码改动，不需测试新增。**

---

### 阶段 6：鼠标滚轮按虚拟行滚动

**文件：** `crates/app/src/app.rs:807`

当前实现按文档行 `scroll_down/up(N)`。改为按虚拟行：

```text
fn scroll_by_visual_lines(&mut self, delta: isize) {
    let Some(dv) = &mut self.doc_view else { return };
    let count = dv.viewport.scroll_line_visual_count;
    if count == 0 {
        // 未知首行虚拟行数（首帧/resize 后），fallback 到文档行滚
        if delta > 0 { dv.scroll_down(delta as usize); }
        else { dv.scroll_up((-delta) as usize); }
        return;
    }
    // delta 步内逐步推进，跨行边界时切换 doc_line 并 reset offset
    // ...
}
```

跨行边界的关键：当 `scroll_visual_offset + 1 >= count` 时，调 `scroll_down(1)`（reset offset = 0，且会在下一帧重算 count）；同理向上跨边界时需要"跳到上一 doc_line 的最后一条虚拟行" —— 这要求知道**上一文档行**的虚拟行数，当前没有缓存。

**简化方案：** 向下跨边界用 `scroll_down(1)` 立即落到下一文档行的首条虚拟行（视觉上等价跳过一行细节，但符合"细粒度滚到下一行第 0 虚拟行"的预期）；向上跨边界（`scroll_visual_offset == 0` 且 `delta < 0`）用 `scroll_up(1)` —— 这会落到上一文档行的**首条**虚拟行，视觉上确实有跳变，但只在跨行那一刻有感。后续优化可以增加 `prev_line_visual_count` 缓存，但**不属于本计划**。

**handle_scroll 改造：** `handle_scroll` 把当前的 `dv.scroll_down/up` 调用替换为 `self.scroll_by_visual_lines(±N)`。

---

### 阶段 7：测试

**文件：** `crates/app/src/viewport.rs`（单元）+ 新增集成测试模块

| 测试名 | 覆盖场景 | 关联阶段 |
|--------|---------|---------|
| `scroll_visual_offset_default_zero` | 新建 viewport 默认 0 | 1 |
| `scroll_visual_line_sets_both_fields` | 同时设置 scroll_line + offset | 1 |
| `scroll_to_resets_visual_offset` | scroll_to 重置 offset | 1 |
| `scroll_up_down_resets_visual_offset` | scroll_up/down 重置 offset | 1 |
| `resize_resets_visual_count_only` | resize 清 count，保留 offset | 1 |
| `set_total_lines_resets_visual_fields` | 文件重载清两字段 | 1 |
| `shape_skips_first_line_visual_offset` | offset > 0 时首行跳过前 N 条虚拟行（集成） | 2 |
| `advance_cache_aligned_with_screen_y` | advance_cache[0] 对应屏幕 y=0 | 2/5 |
| `cursor_visual_line_max_when_in_skipped_area` | 光标在跳过区时 cursor_visual_line == usize::MAX | 2 |
| `scroll_correction_long_line_cursor_below` | 超长行光标在屏幕下方 → 调 offset | 3d |
| `scroll_correction_long_line_cursor_above` | 超长行光标在 skip 区 → 减小 offset | 3c |
| `scroll_to_long_line_then_offset_corrected` | scroll_to 跳到超长行后 offset 自动调整 | 3e |
| `move_cursor_up_into_skipped_area` | 上箭头从可见区进入 skip 区，光标 byte 正确 | 4b |
| `move_cursor_down_past_visible_in_long_line` | 下箭头超出可见区，光标进入超长行下一虚拟行 | 4c |
| `wheel_scroll_step_through_long_line` | 滚轮逐条虚拟行滚过超长行 | 6 |
| `wheel_scroll_fallback_when_count_zero` | 首帧/resize 后 count=0 时 fallback 文档行滚 | 6 |

---

## 影响范围总结

| 文件 | 改动量 | 说明 |
|------|--------|------|
| `viewport.rs` | 中 | 2 个字段 + 2 个新方法 + 5 个现有方法的 reset 行为 + 测试 |
| `app.rs` | 大 | shape_visible_lines（首行 skip + 字段填充）+ 滚动修正块重写 + move_cursor_visual 边界处理 + handle_scroll 改造 + 新增 4 个 self 字段做渲染→输入数据通道 |
| `document_view.rs` | 无 | viewport 已是 `pub` 字段，无需新增 wrapper |
| `input.rs` | 极小 | 仅当 handle_scroll 拆分时需要重新对接（可能不动） |

## 实施顺序

1. **阶段 1** —— viewport 字段与 API，独立可测
2. **阶段 2** —— shape 渲染跳过 + 字段填充。完成后用一份长行测试文件人眼验证
3. **阶段 3** —— 滚动修正逻辑。这一步完成后基本功能可用（不含细粒度滚动）
4. **阶段 4** —— move_cursor_visual 上下箭头精确性
5. **阶段 6** —— 鼠标滚轮（可选/低优先）
6. **阶段 7** —— 各阶段测试随阶段编写

阶段 5 已合并入阶段 2 的 advance_cache 对齐保证，不单列实施。

## 风险点

1. **`cursor_vl_in_doc` 等渲染→输入数据通道的字段一致性**：shape 阶段每帧重写，输入阶段（move_cursor_visual / 滚动修正）读取上一帧的值。键盘事件之间不会跑 shape，所以可能用过期值。需要在每次输入处理后立即触发重绘并 shape，或者在 move_cursor_visual 内部基于当前 viewport 重算（更稳）。**实施时优先考虑后者**。
2. **超长行（虚拟行数 >> visible_rows）**：scroll_visual_offset 可以很大，clamp 上界 = `scroll_line_visual_count - visible_rows`。需要在 scroll_visual_line 中处理（首次进入超长行时 scroll_line_visual_count 可能为 0 ——此时不 clamp，留给下一帧）。
3. **阶段 6 的跨行边界跳变**：向上跨行落到上一文档行首条虚拟行（而非末条），视觉上一次跳一行。如果用户反馈不可接受，再加 `prev_line_visual_count` 缓存优化。
4. **首帧/resize 后 `scroll_line_visual_count == 0`**：所有读这个字段的代码（阶段 6 是主要消费者）都需要处理 0 = 未知，fallback 到文档行级行为。
