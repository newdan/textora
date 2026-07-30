# 滚动两类异常根因分析

> 现象：
> 1. **鼠标滚轮**：滚动时视图区"闪缩"（屏幕短暂收缩 / 显示不一致），但 `scroll_line` 不翻页。
> 2. **方向键 ↓**：状态栏显示的行号在变（说明 cursor 实际在动），但视图区不滚动。
>
> 两者都属于 word-wrap 视图下"viewport 视觉滚动"实现的衍生 bug，与 `plans_viewport_offset_revision.md` 的 B1/B2/B3 漏洞同源，但触发路径不同。本文从代码事实出发拆开两条独立的根因链。

---

## 共同的状态机背景

word-wrap 后，一条文档行可能渲染为 N 条虚拟行。`Viewport` 用两个字段表达视觉滚动位置：

| 字段 | 含义 |
|------|------|
| `scroll_line: usize` | 屏幕首行所在的**文档行**索引 |
| `scroll_visual_offset: usize` | 在 `scroll_line` 内部，跳过前 N 条虚拟行后开始渲染 |
| `scroll_line_visual_count: usize` | `scroll_line` 这条文档行的虚拟行总数（由上一帧 shape 写回；0 表示未知）|
| `visible_rows: usize` | 屏幕能容纳的虚拟行数 |

> 关键不变量：`visible_range() = scroll_line..(scroll_line + visible_rows)`，**仍按文档行粒度返回**。这是后面所有问题的共同基础。

`shape_visible_lines()` 每帧重新对 `visible_range` 内的文档行做 word-wrap，把首行的前 `scroll_visual_offset` 条虚拟行 skip 掉。`shape` 末尾根据 `cursor_visual_line` 与 `visible_rows` 的关系做"滚动修正"，修正不是渲染前生效，而是写回 viewport，依赖下一帧再渲染。

---

## 症状 1：鼠标滚轮 → 视图"闪缩" + 不翻页

### 触发路径
```
WindowEvent::MouseWheel
  → App::handle_scroll(delta)            crates/app/src/app.rs:1184
  → App::scroll_by_visual_lines(±3)      crates/app/src/app.rs:1167
  → 循环 3 次 Viewport::scroll_visual_step_down/up
                                          crates/app/src/viewport.rs:114
```

`scroll_visual_step_down` 的逻辑（`viewport.rs:114-126`）：

```text
count = self.scroll_line_visual_count
if count == 0:
    scroll_down(1)            // 文档行级滚动
    return
max_offset = count.saturating_sub(visible_rows)
if scroll_visual_offset >= max_offset:
    scroll_down(1)            // 跨过当前 doc_line
else:
    scroll_visual_offset += 1 // 同 doc_line 内推进
```

### 根因 1.A：`scroll_line` 指向中等长度 wrap 行时，offset 必须先推满才能翻页

设 `visible_rows = 10`，`scroll_line` 这条文档行的 word-wrap 结果是 12 条虚拟行（`scroll_line_visual_count = 12`）：

- `max_offset = 12 - 10 = 2`
- 用户连续滚轮：每次 `scroll_by_visual_lines(3)` 调三次 `step_down`
  - 第 1 次：`offset 0→1`，`scroll_line` 不变
  - 第 2 次：`offset 1→2`，`scroll_line` 不变
  - 第 3 次：`offset(2) >= max(2)` → `scroll_down(1)`，`scroll_line += 1`，`offset = 0`，`count = 0`
- 看起来没问题。但如果用户**只滚一次**（很多用户的实际节奏）就只走前 1-2 次，`scroll_line` 完全不动 → 用户感觉"未翻页"。

更糟的是，文档里若混着长短行，`max_offset` 在每次 `scroll_down` 后被重置为基于新行的值（`count=0` 时下次走 fallback）。结果是单次滚轮的 `scroll_line` 推进幅度是 0 / 1 / 2 / 3 视具体长度而定，**不可预测**。

### 根因 1.B：`visible_range()` 不补偿 `scroll_visual_offset`，屏幕底部出现空白（闪缩）

`shape_visible_lines` 主循环每帧重建 `advance_cache` 与渲染 vertices：

```text
for i in 0..vis_count:                        // vis_count = visible_range.len() ≈ visible_rows
    visual_lines = wrap(line_bytes, screen_w)
    skip_visual = if i == 0 { scroll_visual_offset } else { 0 }
    render(visual_lines[skip_visual..])       // 首行跳过前 N 条
    visual_line_counter += visual_lines.len() - skip_visual
    if visual_line_counter >= visible_rows && cursor_line_done: break
```

**问题**：`visible_range = scroll_line..(scroll_line + visible_rows)`，长度恒为 `visible_rows`。当 `scroll_visual_offset > 0`：

- 首行只渲染 `visual_lines.len() - offset` 条
- 但末尾 doc_line `scroll_line + visible_rows - 1` 之外的行**不会被带进来**
- 若所有 `visible_range` 内的 doc_line 都是单虚拟行（短行），实际渲染的虚拟行数 = `(N₀ - offset) + (visible_rows - 1) × 1 = visible_rows - offset`
- 屏幕底部留空 `offset` 行 → 用户视觉上看到的"屏幕缩小了 offset 行"

### 根因 1.C：`advance_cache` 越界与帧间不一致（闪烁）

`shape` 主循环把整条 `visual_lines[skip_visual..]` 都 push 进 `advance_cache`，没有按 `visible_rows` 截断（`plans_viewport_offset_revision.md` 的 B2 漏洞），每帧 `advance_cache` 长度随首行 wrap 数剧烈变化：

| 帧 | scroll_line | offset | 首行 vl 数 | advance_cache 长度 |
|----|-------------|--------|----------|-------------------|
| t  | 5           | 0      | 1        | ≈ visible_rows = 10 |
| t+1 (滚轮一下到中长行) | 6 | 0 | 12 | 12 + 后续 |
| t+2 (再滚) | 6 | 1 | 12 | 11 + 后续 |
| t+3 (再滚) | 7 | 0 | 1  | ≈ visible_rows |

加上根因 1.A 的"offset 先推满"延迟，相邻帧 `advance_cache` / 渲染范围频繁跳变，且 `cursor_visual_line` 用旧值渲染（B1 漏洞，见症状 2 的根因 2.B），用户看到的就是**滚轮转一下，屏幕短暂闪一下、内容跳一下、却没翻页**。

### 修复方向（仅供参考，非本次任务）
- 让 `visible_range()` 在 `scroll_visual_offset > 0` 时多带一行 doc 进来（`end = scroll_line + visible_rows + 1`），保证 `Σ(渲染 vl 数) ≥ visible_rows`，消除空白。
- `shape` 主循环加 `if visual_line_counter >= visible_rows + cursor_safe { break }`，让 `advance_cache` 长度稳定。
- 滚轮跨 doc 边界时，把 `delta` 中已用掉的步数减去 `max_offset - cur_offset`，剩余 step 在新行继续推（让 1 次滚轮永远滚 N 条虚拟行，不被中长行"吞 step"）。

---

## 症状 2：方向键 ↓ → 行号变，视图不动

### 触发路径
```
WindowEvent::KeyboardInput(ArrowDown)
  → key_to_command → EditCommand::MoveDown
  → App::handle_command                 crates/app/src/app.rs:1213
  → App::move_cursor_visual(1)          crates/app/src/app.rs:334
       ├─ 4a: target 在 advance_cache 范围内 → cursor_move_to_offset(...)
       ├─ 4b: target 越界向上 → 滚 offset / 移光标
       └─ 4c: target 越界向下 → 滚 offset / 移光标
  → next frame: shape_visible_lines 末尾的"滚动修正"块
                                          crates/app/src/app.rs:1019-1053
```

### 根因 2.A：`move_cursor_visual` 只动光标 byte，不动 viewport

按 ↓ 一次的常见路径是 4a（`target_vis < advance_cache.len()`，`app.rs:338-365`）：

```text
let (doc_line, clusters) = &advance_cache[target_vis];
cursor_move_to_offset(line_start + best_by_sticky_x);
// scroll_line / scroll_visual_offset / scroll_line_visual_count 全部不变
```

`advance_cache` 在根因 1.C 里已说明可能"超出屏幕长达数十条虚拟行"。所以 `target_vis = current + 1` 完全可能落在屏幕外的 cache 项上 —— **光标 byte 跳到了屏幕外，但 viewport 这一帧不动**。`needs_redraw = true`，等下一帧 shape 自我修正。

### 根因 2.B：shape 末尾的"滚动修正" else 分支算错

下一帧 `shape_visible_lines` 末尾（`app.rs:1017-1053`）：

```text
if cursor_doc_line < range.start:                       // 3a
    scroll_to(cursor_doc_line)
elif cursor_doc_line >= range.end:                      // 3b
    scroll_to(cursor_doc_line - (visible_rows - 1))
elif cursor_visual_line == MAX && cursor_doc_line == scroll_line:   // 3c
    scroll_visual_offset = cursor_vl_in_doc
elif cursor_visual_line != MAX && cursor_visual_line >= visible_rows: // 3d
    if cursor_doc_line == scroll_line && doc_visual_count > visible_rows:
        scroll_visual_offset = (offset + 1).min(max_offset)
    else:
        scroll_to(cursor_doc_line.saturating_sub(visible_rows - 1))
```

**Bug 出现在 3d 的 else 分支**。复现配置：

- `scroll_line = 5`，doc_line 5 是 5 vl 短行，doc_line 6 是 30 vl 长 wrap 行
- `visible_rows = 10`，屏幕首行起：5 (5 vl) + 6 (前 5 vl) = 10 行恰好填满
- 光标在 doc_line 6 第 4 条虚拟行（cursor_visual_line = 5+4 = 9，屏幕内）

按 ↓ 一次：
1. 4a：把光标移到 `advance_cache[10]` —— doc_line 6 的第 5 条虚拟行（屏幕外）
2. 下一帧 shape：`cursor_doc_line = 6`，`cursor_vl_in_doc = 5`，`cursor_visual_line = 5 + 5 = 10`
3. `10 >= visible_rows(10)` → 进 3d
4. `cursor_doc_line(6) != scroll_line(5)` → 走 **else** 分支
5. `scroll_to(6.saturating_sub(9)) = scroll_to(0)` —— **scroll_line 反向跳到 0**！
6. `scroll_to` 同时 reset `scroll_visual_offset = 0`、`scroll_line_visual_count = 0`

第三帧 shape：

- `range = 0..10`
- `cursor_doc_line = 6` 仍在 `range` 内
- `i = 0..6` 全 shape；i=6 处理 cursor 行：`cursor_visual_line = (vl 0..5 累计) + 5`
- 假设 doc_line 0..4 均单 vl、doc_line 5 是 5 vl，前 6 行共 10 vl → cursor_visual_line = 10 + 5 = 15
- 依然 `15 >= 10` → 3d 的 else 又触发：`scroll_to(6 - 9) = scroll_to(0)` → **不变**

**死循环**。每帧滚动修正都试图把 cursor_doc_line 放到屏幕底部（`scroll_to(cursor_doc_line - (visible_rows - 1))`），但忽略了：

- cursor_doc_line 之前的 doc_line 可能也是 wrap 行，累计虚拟行数早就吃掉了 `visible_rows`
- `saturating_sub` 让 `cursor_doc_line < visible_rows - 1` 时直接归 0，反而把视图拉到文档开头

外加 `cursor_vertices()` 用 `cursor_visual_line = 15` 算 `line_y = 15 × line_height`，远在屏幕底部之外 → 不显示光标。

**用户观察到的就是**：

- 状态栏的 line 号在变（`dv.cursor_offset` 真的推进了）
- 屏幕完全不滚（`scroll_line` 卡在 0 或某固定值，`scroll_to` 反复 no-op）
- 光标不见了（渲染到屏幕外）

### 根因 2.C：`cursor_visual_line` 在 cursor 离开 viewport 时不更新

补充根因（plans_viewport_offset_revision.md 的 B1）：`shape_visible_lines` 内只有 `if i == cursor_vis_line` 分支会写 `cursor_visual_line`。当根因 2.B 的 `scroll_to(0)` 让 cursor 落入 `range_pre`（0..10）但累计 vl 已超 `visible_rows` 时，`i == cursor_vis_line` 仍会被命中并写入 15，但**渲染层不显示**这条；又因为代码 line 745 的提前置 MAX 判定只看 doc_line range，不看 visual line —— 所以 cursor_visual_line 写回了一个屏幕外的值，下游 `cursor_vertices` 老老实实把 caret 画到屏幕外。配合 2.B，光标"消失"。

### 修复方向（仅供参考，非本次任务）
- 3d 的 else 分支不应该 `scroll_to`，而应该考虑 cursor_doc_line 之前所有 visible 行的累计 vl 数；让"目标"是 `scroll_visual_offset` 推进若干步，让 `cursor_visual_line` 落回 `[0, visible_rows)`。
- 或者把 3d 改成：先把 cursor_doc_line 滚到首行（`scroll_line = cursor_doc_line`），再依靠 3c 调 `scroll_visual_offset = cursor_vl_in_doc.saturating_sub(visible_rows - 1)`。
- shape 入口处除了判 doc_line 范围，还要判 cursor_visual_line 是否会落在 `[0, visible_rows)`，否则置 MAX 阻断错位渲染。

---

## 两个症状的共同症结

| 共同症结 | 影响症状 1 | 影响症状 2 |
|---------|----------|----------|
| `visible_range()` 按 doc_line 粒度返回，不补偿 `scroll_visual_offset` | 屏幕底部留白（闪缩） | 影响 3d 修正时累计 vl 估算 |
| `advance_cache` 不在 `visible_rows` 处截断 | 帧间长度抖动（闪烁） | 4a 把光标移到屏幕外 cache 项 |
| 滚动修正只走单步推进 | 多次滚轮才翻 1 页 | 3d else 分支用错误公式 → 死循环 |
| `cursor_visual_line` 失效情形未全盖 | 闪烁中可能误绘 | 光标消失 |

修复优先级建议沿用 `plans_viewport_offset_revision.md` 的 P0/P1 表，加一项：**3d 的 else 分支重写**（症状 2 的核心 bug，未在原修订计划中识别）。
