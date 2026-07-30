# Viewport 虚拟行偏移 — 实现复审与修订方案

> 基于当前 working tree 与 `plans_viewport_visual_offset.md` 比对。`cargo build` / `cargo test` 全绿，242 passed。下面列出实现与计划的偏离及新发现的设计漏洞。

---

## A. 与计划不符的实现 Bug

### A1. `scroll_to / scroll_up / scroll_down` 未重置 `scroll_line_visual_count`

**位置：** `crates/app/src/viewport.rs:57-75`

```rust
pub fn scroll_down(&mut self, delta: usize) {
    self.scroll_line = self.scroll_line.saturating_add(delta);
    self.scroll_visual_offset = 0;
    self.clamp();
}
```

计划阶段 1 明确要求：「`scroll_to / scroll_up / scroll_down`：在内部调 `clamp()` 之后，重置 `scroll_visual_offset = 0`、**`scroll_line_visual_count = 0`**」。实现只清了 `scroll_visual_offset`，没清 `scroll_line_visual_count`。

**后果：** 连续鼠标滚轮跨 doc_line 边界时 —— `scroll_visual_step_down` 触发 `scroll_down(1)` 把 scroll_line 推进到下一行，`scroll_line_visual_count` 仍是**上一行**的值。下一次 `scroll_visual_step_down` 用过期 count 算 `max_offset = old_count - visible_rows`，可能：
- 旧行是超长行（count=30，max=20），新行是短行（实际 count=1）；step_down 看到 offset=0 < max=20，把 offset 推到 1 —— 但新 doc_line 实际只有 1 条虚拟行
- 下一帧 shape 进入 `i == 0`，`skip_visual = 1`, `visual_lines.len() = 1`，`visual_lines[skip_visual..]` 为空，首行整行不渲染、不进 advance_cache
- 屏幕第一行变成下一 doc_line（看起来 OK），但 `cursor_visual_line` 不更新，`cursor_visual_line_in_doc` 也不更新，3c/3d 可能误判

**修复：** 在 `scroll_down/up/to` body 末尾加 `self.scroll_line_visual_count = 0;`。`set_total_lines` 已正确重置，作参考。

### A2. `move_cursor_visual` 4b 不移动光标，只滚动视口

**位置：** `crates/app/src/app.rs:344-356`

```rust
} else if target_vis < 0 && !self.advance_cache.is_empty() {
    let (first_doc_line, _) = self.advance_cache[0];
    if first_doc_line == ... && ... .scroll_visual_offset > 0 {
        if let Some(dv) = self.doc_view.as_mut() {
            dv.viewport.scroll_visual_offset -= 1;
            // For now, keep cursor at the same byte offset (it's still on the same doc line).
        }
    } else if first_doc_line > 0 { ... }
}
```

**问题：** 上箭头从可见区进入 skip 区时，只把 `scroll_visual_offset -= 1`，**光标 byte offset 完全不变**。光标视觉上原地不动，但视口往上滚了一行。用户期望"光标向上移动一条虚拟行"，结果是"光标不动、视口反向滚动"。

更糟的是：下一帧 shape 跑完后，`cursor_visual_line` 仍指向原 cursor_col 所在的虚拟行（即原视觉位置 + 1，因为视口下移了一行），3d 又会触发滚动修正把 offset 加回去 —— **可能死循环或抖动**。

**修复：** 进入此分支时需要：
1. 拿到 `first_doc_line` 的完整 word wrap 结果（visual_lines）
2. 找到第 `scroll_visual_offset - 1` 条虚拟行的 cluster 范围
3. 用 `sticky_x` 在该条虚拟行的 cluster advances 中选最近列，计算目标 byte offset
4. `dv.cursor_move_to_offset(line_start + best_offset)`

由于 word wrap 在 shape 阶段才有，需要新增字段 `first_line_visual_lines: Vec<(usize, usize, f32)>`（plan 提过但实现没做），由 shape 阶段写入。或者在 `move_cursor_visual` 内部对 `first_doc_line` 重新跑 word wrap（输入处理时无法访问 `text.shaper`，开销也大）。建议字段缓存方案。

### A3. `move_cursor_visual` 4c 不使用 sticky_x

**位置：** `crates/app/src/app.rs:371-413`

```rust
let next_byte_in_line = last_clusters.last().map(|c| c.0).unwrap_or(0);
// ...
let target = line_start + next_byte_in_line;
// ...
if is_long_first_line && target < line_end {
    if let Some(dv) = self.doc_view.as_mut() {
        dv.viewport.scroll_visual_offset += 1;
        dv.cursor_move_to_offset(target);
    }
}
```

**问题：** 下箭头跨过可见区时，光标落在"当前末尾虚拟行最后 cluster 的下一字节"，即下一虚拟行的开头 —— **完全忽略 sticky_x**。用户的预期是：垂直方向移动应保持视觉列（sticky_x），可现在不管之前在哪一列，下箭头超出屏幕都会跑到下一虚拟行的第 0 列。

**修复：** 同 A2，需要 `last_line_visual_lines` 字段，对下一虚拟行用 sticky_x 选列。当下一虚拟行属于下一 doc_line 时，需要先 shape 那个 doc_line 才能选列 —— 退化为"跳到下一 doc_line 开头"也勉强可接受，但同 doc_line 内的下一虚拟行（超长行场景）必须 sticky_x。

### A4. `move_cursor_visual` 4b 中的 `dv` 命名变量未被使用

**位置：** `crates/app/src/app.rs:351-356`

```rust
if let Some(dv) = self.doc_view.as_mut() {
    dv.viewport.scroll_visual_offset -= 1;
    // For now, keep cursor at the same byte offset...
}
```

绑定了 `dv` 但只用了一行字段写入。Rust 不会警告（被使用了一次），但读起来诡异。修 A2 后会自然消失。

---

## B. 设计漏洞（plan 未充分考虑）

### B1. `cursor_visual_line` / `cursor_visual_line_in_doc` 在 cursor 出 viewport 时不更新

**位置：** `crates/app/src/app.rs:611` (`if i == cursor_vis_line { ... }`)

`cursor_visual_line` 和 `cursor_visual_line_in_doc` 这两个字段**只在** shape 循环命中 cursor 所在 doc_line 时被写入。当 cursor doc_line 不在 visible_range（即 `cursor_vis_line == usize::MAX`），整个 `for i in 0..vis_count` 循环都不会进入 `i == cursor_vis_line` 分支，两个字段保持上一帧的值。

**后果链：**
- 滚动逻辑 3a/3b（`cursor_doc_line < range.start` 或 `>= range.end`）只走 `scroll_to`，**不读** cursor_visual_line —— 这部分安全
- 但 `cursor_vertices()`（app.rs:438-440）使用 `cursor_visual_line` 计算 y 位置渲染 caret —— 当 cursor 在视口外时，这帧会**画一个错位的 caret**（用过期值）。下一帧 scroll 修正后才正确。短暂闪烁

**进一步：** 实现里 `cursor_visual_line == usize::MAX` 时 `cursor_vertices` 直接 `return vec![]`（line 427）—— 这是好的，可以避免错位渲染。但仅当上一帧 cursor 已经在 skip 区时才有 MAX 这个值；当 cursor doc_line 完全在视口外（上一帧 cursor 在视口里、normal 值），本帧 cursor_visual_line 仍是个**合法的小整数**，会走渲染路径。

**修复：** 在 shape_visible_lines 进入主循环前，先判断 `cursor_doc_line` 是否在 `visible_range()` 内；若不在，立刻 `self.cursor_visual_line = usize::MAX;` 防止渲染错位。

### B2. `visible_range()` 在 `scroll_visual_offset > 0` 时低估屏幕容量

**位置：** `crates/app/src/viewport.rs:78-82`

```rust
pub fn visible_range(&self) -> std::ops::Range<usize> {
    let start = self.scroll_line;
    let end = (self.scroll_line + self.visible_rows).min(self.total_lines);
    start..end
}
```

**问题：** 当 `scroll_visual_offset > 0` 时，首行只渲染 `total_visual_in_first - scroll_visual_offset` 条虚拟行，这通常少于"完整一行"，因此屏幕末尾**会有更多 doc_line 可以塞入**。但 `visible_range()` 仍按 doc-line 粒度返回 `scroll_line..(scroll_line + visible_rows)`，shape 主循环遍历到 `vis_count = visible_range.len()` 就停了，不会迭代到屏幕底部应显示的额外 doc_line。

**复现：** scroll_line=5, scroll_visual_offset=3, visible_rows=10, doc_line 5 有 5 条虚拟行（其中 2 条可见）；doc_line 6..14 都是单行。屏幕预期填满 10 行虚拟行 = 2 (line 5) + 8 (line 6..13) = 10 行，但 `visible_range()` 返回 `5..15`，shape 只迭代到 doc_line 14（含），最后两条虚拟行其实够；但若 doc_line 5..15 总虚拟行 < 10，**屏幕底部会有空白**。

更明显的反例：scroll_line=5, scroll_visual_offset=3, doc_line 5 是超长行有 30 条虚拟行 → 屏幕全被 doc_line 5 填满 27 条虚拟行（30-3），visible_range=5..15 让 shape 迭代到 doc_line 14。这种情况浪费迭代但视觉无 bug —— 因为前 visible_rows 条虚拟行已经填满，render 在 visual_line_counter 累加的过程中应该能在 visible_rows 处自然停下。但实际代码**没做"填满 visible_rows 就停"的检查**：循环把所有 visible_range 内的 doc_line 都 shape 一遍并 push 到 advance_cache。

**后果：** advance_cache 末尾会塞入超出屏幕的虚拟行。`hit_test` 用 `py / line_height` 索引时不会越界（hit_test 检查 `vis_line >= advance_cache.len()`）—— 但点击屏幕底部空白区会**误命中**屏幕外的虚拟行。

**修复：** 两个方向之一：
1. shape 主循环在 `visual_line_counter - skip_visual >= visible_rows` 时 break，不再迭代后续 doc_line
2. `visible_range()` 不变，但在 advance_cache 写入时按 visible_rows 截断

方案 1 更省 shape 开销，推荐。

### B3. 阶段 3a/3b 不处理"跳转到超长行中间"

**位置：** `crates/app/src/app.rs:794-801`

```rust
if cursor_doc_line < range.start {
    dv.viewport.scroll_to(cursor_doc_line);
} else if cursor_doc_line >= range.end {
    dv.viewport.scroll_to(cursor_doc_line.saturating_sub(visible_rows.saturating_sub(1)));
}
```

`scroll_to` 会重置 `scroll_visual_offset = 0`，所以跳转到一条 30 行虚拟行的超长行后，cursor 在第 25 条虚拟行 → 本帧 viewport 显示首行 0..10 条虚拟行 → cursor 不在屏幕上 → 下一帧 shape 跑出 `cursor_visual_line == ?`（cursor_vl_in_doc=25, skip=0, cursor_visual_line = 25），3d 触发 `scroll_visual_offset = 25 - 9 = 16` → 第三帧才稳定。

**后果：** 跳转动作（搜索、Ctrl+G、文件打开后落在某行）需要 3 帧才稳定，第 1-2 帧 cursor 不可见或位置错误。

**修复：** 3a/3b 内部直接计算超长行情况：
```rust
if cursor_doc_line >= range.end {
    let count_at_target = ...; // 需要：知道 cursor 所在 doc_line 的 visual_lines.len()
    if count_at_target > visible_rows {
        dv.viewport.scroll_visual_line(cursor_doc_line, cursor_vl_in_doc.saturating_sub(visible_rows - 1));
    } else {
        dv.viewport.scroll_to(cursor_doc_line - visible_rows + 1);
    }
}
```

但这需要在跳转的当帧就有"目标 doc_line 的虚拟行数" —— 同 plan 风险点 1：键盘事件之间不 shape，没有这个值。**折衷方案：** 接受 2-3 帧稳定，但在第 1 帧把 `cursor_visual_line` 设 `usize::MAX` 防止画错位 caret（与 B1 修复一并）。

### B4. plan 阶段 4 提到的 `first_line_visual_lines` / `last_line_visual_lines` 未实现

**位置：** 缺失

A2、A3 都依赖这两个字段。当前实现取了简化路径（不动光标 / 不用 sticky_x），结果是 4b/4c 的功能不完整。

**修复：** 在 `App` struct 上新增：
```rust
first_line_visual_lines: Vec<(usize, usize, f32)>,  // 首行 word wrap 结果
last_line_visual_lines: Vec<(usize, usize, f32)>,   // 末行 word wrap 结果
first_line_clusters: Vec<shaping::Cluster>,         // 给 sticky_x 选列用
last_line_clusters: Vec<shaping::Cluster>,
```

shape 主循环在 `i == 0` / `i == vis_count - 1`（或 cursor 邻接行）时把这些写入。`move_cursor_visual` 读取后做精确定位。

---

## C. 小问题

### C1. `Viewport::resize` 未重置 `total_visual_lines: Option<usize>`

**位置：** `crates/app/src/viewport.rs:47-54`

`resize` 改变 `visible_rows`，间接改变 word wrap 结果（`viewport_width` 跟着窗口宽度变 → 一行换不换行的判定变 → 总虚拟行数变）。但 `total_visual_lines: Option<usize>` 是滚动条/全局虚拟行数估算，resize 后未重置，估算过期。

这是历史接口的问题，不属于本计划的核心逻辑，但顺手修一下：
```rust
pub fn resize(&mut self, visible_rows: usize) {
    self.visible_rows = visible_rows.max(1);
    self.scroll_line_visual_count = 0;
    self.total_visual_lines = None;  // 新增
    self.clamp();
}
```

### C2. clippy 警告：collapsible if

```
warning: this `if` statement can be collapsed
```

未定位具体行，运行 `cargo clippy --fix --lib -p edit-plus-app` 即可。

### C3. plan 风险点 1 没有缓解措施

> 键盘事件之间不会跑 shape，所以可能用过期值。需要在每次输入处理后立即触发重绘并 shape，或者在 move_cursor_visual 内部基于当前 viewport 重算（更稳）。

实现选择了"用上一帧字段"，没做内部重算，也没做"输入后强制 shape"。当前 4b/4c 简化版意外地避开了大部分老化字段读取（4b 只读 scroll_line/scroll_visual_offset，4c 只读 scroll_line_visual_count），但一旦修了 A2/A3，对 `first_line_visual_lines` 的读取就会撞上时序问题。

**建议：** 实施 A2/A3 时同步在 shape 末尾把 `cursor_*` / `first_line_*` / `last_line_*` 字段一次性 commit；输入处理读取这些字段视为"上一帧的快照"，在 4b/4c 内部不做 shape 重算（性能/复杂度折衷）。配合 B3 接受多帧稳定。

---

## D. 修复优先级

| 等级 | 项 | 说明 |
|------|-----|------|
| P0 | A1 | 一行修复，避免连续滚轮在跨 doc 边界后行为错乱 |
| P0 | B1 | 两行修复，避免视口外 cursor 渲染错位 caret |
| P1 | A2 + A3 + B4 | 配套：新增 `first_/last_line_visual_lines` 字段 + sticky_x 选列。一并实施 |
| P1 | B2 | shape 主循环在 visible_rows 满时 break，避免 hit_test 误命中 |
| P2 | B3 | 跳转场景多帧稳定问题，可以接受现状或优化为单帧 |
| P3 | C1, C2, C3 | 卫生与文档 |

## E. 测试补强

修复后需要新增/补强的测试：

| 测试 | 覆盖 |
|------|------|
| `scroll_down_resets_scroll_line_visual_count` | A1 |
| `wheel_scroll_short_to_long_doc_line_no_glitch` | A1 集成路径 |
| `move_up_into_skipped_area_moves_cursor_byte` | A2 —— 不只是滚视口 |
| `move_down_past_visible_preserves_sticky_x` | A3 |
| `cursor_invisible_when_doc_line_out_of_viewport` | B1 |
| `visible_range_fills_screen_with_offset` | B2 |
| `goto_long_line_middle_no_caret_flash` | B1 + B3 |

---

## F. 实施建议顺序

1. **P0 修复**（A1 + B1）—— 各 1-2 行，立刻消除两类已知错位
2. **B2 修复** —— shape 主循环加 break 守卫，连带把 advance_cache 写入也守住
3. **P1 包**（A2 + A3 + B4）—— 一起做，引入 `first_/last_line_visual_lines` 字段
4. **测试补强** —— 跟随每个 P0/P1 修复
5. **B3 优化**（可选）—— 若用户反馈跳转闪烁不可接受再做
6. **C1/C2/C3** —— 杂项清理
