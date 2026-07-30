# 幽灵空行（Ghost Lines）根因分析 v2

> 现象：滚动 `crash.log` 时行号从 161 直接跳到 164，中间 162、163 行号不显示，也不折行，等待几秒后恢复。
> 触发文件：`/Users/dan/proj/llmws/edit+/crash.log`（756 行，行 553 为 69086 字节超长行）
> 现状：HEAD = `bbc766b`（revert 了 `pending_scroll_adjust` 机制）+ `b56637a`（placeholder skip 渲行号）+ `9ea35cc`（max_line_bytes_for_shaping=5000）。

---

## TL;DR

最新一次"应当修复"的提交链没有真正解决幽灵行：

- `b56637a` 给 placeholder skip 路径补了行号渲染；
- `9ea35cc` 把 worker 单行 shape 量限到 5000 字节；
- `bbc766b` 撤回了 `pending_scroll_adjust`，理由是把根因归到 placeholder skip。

但用户在 HEAD 上仍能看到 162/163 行号缺失。原因不止一个：

1. **placeholder skip 阈值不再触发**（`max_sync_bytes = 4000`，最长一行才 235 B），所以 b56637a 的行号补丁对这几行不生效——这条路径根本没走。
2. **真正吞掉 162/163 行号的路径**，是 shape 主路径里 `if line_bytes.is_empty() { continue; }`（`render_pipeline.rs:335`）：当 `dv.doc_line_bytes(dl)` 因为 LineIndex 偏移过期返回空切片时，`continue` 直接吞掉**整个 doc line 的渲染**——既不画行号、也不增量 `visual_line_counter`、也不 push advance_cache。下一次 i 用错位的 `visual_line_counter` 继续画，前一行的 Y 留白即"无行号无内容的空带"。
3. **bbc766b 撤回 anchor sync 反而让窗口更脆**：worker 在背景把行 553 从 estimated `vl_count ≈ 519` 写回到真实值（在 actual char_width 下可能更大或更小），`drain_reshape_results` 触发 `rebuild_tree + restore_scroll_from_anchor + clamp_scroll_top`。但此时 anchor 在上一帧基础上由旧 tree 推得，`restore` 会让 `scroll_top` 跳一档；接着 shape_visible_lines 用新 scroll_top + 新 tree 重新做 `display_to_doc(scroll_top.floor())`，可见 doc_line 集合已经偏移。如果在偏移生效那一帧某个 `doc_line_bytes` 拿到错的偏移→空切片→走 `continue`→幽灵行出现，**直到 worker 队列清空、scroll_anchor 自己稳定**，所以"等几秒就好"。

幽灵行有**多个并存的根因路径**，b56637a 只把其中"placeholder skip 不画行号"这条堵住，没堵其它两条。

---

## 一、当前 HEAD 的代码结构（事实）

| 模块 | 关键代码 | 当前行为 |
|---|---|---|
| `app.rs:227-247` `init_display_map` | placeholder 用 `(len / (vp_width / (font*0.6))).ceil()` 估算 vl_count | 行 553 estimate 远低于真实值（ASCII 0.6em 估出来约 519，真实可能 800+） |
| `app.rs:1312-1347` `drain_reshape_results` | tree_dirty → `rebuild_tree` + `restore_scroll_from_anchor` + `clamp_scroll_top` | **立即调整 scroll，未在帧首/帧末协调，可能与本帧 `shape_visible_lines` 互相踩** |
| `app.rs:1485-1494` `render` | `tree_dirty=true → restore + clamp` | 与 `drain_reshape_results` 是相同代码，但在本帧晚些时候再跑一次 |
| `render_pipeline.rs:330-335` shape 主路径入口 | `let line_bytes = dv.doc_line_bytes(dl); ... if line_bytes.is_empty() { continue; }` | **悄悄吞掉整个 doc line，不画行号、不进 advance_cache、不增 counter**——这是关键漏洞 |
| `render_pipeline.rs:340-386` placeholder 长行 skip | `is_placeholder && line_len > 4000` | crash.log 全部行 ≤ 235 B，**永远不进这一支** |
| `render_pipeline.rs:823-827` shape 主路径 entry 写回 | `update_entry_in_place(...)` + `tree_dirty_local = old_vl_count != new` | 一次循环可能多次置 `tree_dirty_local=true`；循环末 `rebuild_tree()` |
| `display_line_map.rs:88-107` `set_viewport_size` | width 变 ±1 / font 变 ±0.1 → `entries[*].visual_breaks.clear(); visual_line_count = 1; tree = from_entries(clone)` | **每帧 `shape_visible_lines` 第一行就调用一次**；阈值若被亚像素抖动击穿，整树全部清空、当前帧 wrap 全部失效，下一帧才被 worker / sync shape 逐行重建 |

---

## 二、复现链路（基于 161→164 现象的推理）

**前置条件**：crash.log 已打开；用户首次滚动到 161 附近。

### 时刻 T0 — init_display_map 后第一次 render

- `entries[i].visual_line_count` 全是 estimate（多数行 1，行 553 被估为 ≈519）。
- `tree.total_rows()` ≈ 文件总行数 + 行 553 多出的 estimate。
- `display_to_doc(scroll_top.floor())` 给出某个 doc_line（比如 161）。

### 时刻 T1 — submit_reshape_ahead 提交 worker

- `app.rs:1356-1396` 给 `range.start - 64 .. range.end + 64` 范围内每行提交 worker 请求。
- worker 在后台 shape 后回写真实 vl_count。

### 时刻 T2 — drain_reshape_results 在某一帧到达

- 某行（最可能是行 553）的真实 vl_count 与 estimate 不同 → `tree_dirty = true`。
- `rebuild_tree()` → `restore_scroll_from_anchor + clamp_scroll_top`。
- restore 的语义是 `scroll_top = doc_to_display(anchor.doc_line) + pixel_offset / line_height`。
  - anchor 是上一帧由 `clamp_scroll_top → sync_anchor_from_scroll` 写入的：`(doc_line=display_to_doc(scroll_top.floor(), T_old), pixel_offset=fract * lh)`。
  - 现在 tree 是 `T_new`，`doc_to_display(anchor.doc_line, T_new) ≠ doc_to_display(anchor.doc_line, T_old)`。
  - 结果：`scroll_top` 跳变，差值与"行 553 estimate 与 actual 之差"成正比。

### 时刻 T3 — 同一帧紧接着 shape_visible_lines

- `set_viewport_size(...)` 没改宽度，跳过 invalidate。
- 用**新 scroll_top + 新 tree** 算 `start = display_to_doc(scroll_top.floor())`。
- 新 start 可能是 162，但循环里 `range_pre.start + i` 落到的某行 `dv.doc_line_bytes(dl)` 可能因为 LineIndex 与 worker 写回的 `byte_offset` 不一致返回空切片（`render_pipeline.rs:330-334`），命中 `continue`：
  - 行号不画。
  - `visual_line_counter` 不增。
  - `advance_cache` 不 push。
- 视觉上：上一行 Y 之后留白一段，没有行号，再画下一行。**这是用户看到的"无行号空带"**。

### 时刻 T4 — 之后几帧

- worker 持续把 estimated 行变 actual，每次 `tree_dirty` 又触发一次 `restore + clamp`。
- 但**每次 restore 之后 clamp 内部都会再 sync_anchor**，下一帧 anchor 已经基于新 tree。
- 当 worker 队列排空、tree 不再变化时，anchor 不再漂移，shape_visible_lines 拿到正确的 start，`doc_line_bytes` 命中正常字节，幽灵行消失——**这就是"等几秒就好"**。

---

## 三、关键嫌疑点逐条验证

### S1. `line_bytes.is_empty()` 静默 continue（**最可疑，必须修**）

`render_pipeline.rs:330-335`：

```rust
let line_bytes = {
    let dl = range_pre.start + i;
    dv.doc_line_bytes(dl)
};
let Some(line_bytes) = line_bytes else { continue };
if line_bytes.is_empty() { continue; }
```

两个 `continue` 都没有：
- 渲染行号；
- 增量 `visual_line_counter`；
- push advance_cache 占位。

后果：
- **行号缺失**——刚好是用户看到的现象。
- **后续行 Y 坐标错位**——`visual_line_counter` 没增，下一行画在了"应该是这一行"的 Y 上，但 `display_to_doc` 给出的 doc_line 已经跳过了这一行。视觉上是"中间留白 + 行号跳号"。
- **advance_cache 索引与 visual_line 错位**——`first_line` / `last_line` / hit-test 拿到错位数据，后续光标列错乱（B-6 同源）。

**注意**：上面这条第二个 `continue`（`line_bytes.is_empty()`）是合法的——空 doc 行有专门的早期分支（`render_pipeline.rs:145-193`）。但 `length == 0` 是用 `line_byte_length` 判断的，`line_bytes.is_empty()` 是用 `doc_line_bytes` 返回值判断的，两者**应当**等价，但实际可能因 LineIndex 与 buffer 不同步而出现 `length > 0 && line_bytes empty` 的窗口（特别是切 tab、open file、worker 写回交叉时）。

### S2. `length == 0` 早期 continue 的不对称

`render_pipeline.rs:144` 的 `let length = if let Some(l) = length { l } else { continue; };`——**这条 continue 也吞行号、不增 counter、不 push advance_cache**。

length 来自 `dv.line_byte_length(doc_idx)`，它在 LineIndex 没建好时返回 None。crash.log 不会触发，但快速 open_file 后第一帧可能命中。

### S3. `set_viewport_size` 容差与每帧调用

`render_pipeline.rs:76` 每帧 `display_map.set_viewport_size(screen_w - scrollbar_reserve - left_margin, font_size)`。

- 容差 width ±1.0、font ±0.1。
- crash.log 文件行号位数为 3（max 756），`gutter_width` 在 line_count 跨过 1000 时会从 3 → 4 位扩张。756 行不会跨过。但**渲染开始时 `dv.line_count()` 可能因为缓冲区还在 rebuild 短暂为 0** → `gutter_width(0)` 走 `digits=3` 同值。这一支应该 OK。
- 但 `content_left_margin().max(gutter_width(line_count))` 在 dpi_scale 短暂改变时（如外接屏拖拽）会突变 → 触发 invalidate → 全文 vl_count 归 1 → **scroll_top 在新 tree 里指向另一个 doc_line** → 同一帧 shape 的 range 完全错位 → 幽灵带。
- `bbc766b` 撤掉的 `pending_scroll_adjust`正是为了避免这种"同帧 tree 已变、vertices 已生成"的不一致。

### S3.1 `set_viewport_size` 与 `scrollbar_reserve` 同帧不同步

`render_pipeline.rs:76` 用 `Settings::get().scrollbar_reserve()`。
`render_pipeline.rs:198,458` 也用 `Settings::get().scrollbar_reserve()`。
3 处分别从 `Settings::get()` 拿，**理论上同一帧拿同一个值**，但 `Settings::get()` 内部是 thread_local Ref；如果 `dpi_scale` 在帧中被 ScaleFactorChanged 改了一次（`394a7f4` 修过 idempotent，但 set_dpi_scale 仍可写），值会变。本机典型场景不会触发，可以暂忽略。

### S4. `restore_scroll_from_anchor` 漂移（撤回的 P0）

`bbc766b` 撤掉了 `e580187` 在 `tree_dirty` 块内加的 `sync_anchor_from_scroll`。

- 撤回理由："根因是 placeholder skip"——但事实是 placeholder skip 路径在 crash.log 短行场景**根本不触发**（max_sync_bytes=4000，最长行 235 B）。
- 撤回带回了原始漂移：`restore_scroll_from_anchor` 用旧 anchor + 新 tree → scroll_top 跳变。
- `restore` 后立即 `clamp_scroll_top` → 内部 `sync_anchor_from_scroll` 用新 scroll_top + 新 tree 写回 anchor，下一帧 anchor 是对的——但**当帧 shape 已经用错位 scroll 跑了一次**。

`drain_reshape_results` 里的"立即 restore + clamp"和 `render` 里的"shape 后 restore + clamp"**重复且彼此竞争**：

```text
帧 N:
  drain_reshape_results:
    rebuild_tree                  # tree T_N
    restore_scroll_from_anchor    # 用 anchor_(N-1)（基于 T_(N-1)）→ scroll_top 漂移
    clamp_scroll_top              # 内部 sync_anchor → anchor_N（基于 T_N）

  shape_visible_lines:
    set_viewport_size             # 通常不动
    start = display_to_doc(scroll_top.floor(), T_N)  # 用漂移后的 scroll_top
    循环渲染 → 触发 update_entry_in_place
    rebuild_tree                  # tree T_N'
    tree_dirty_local = true → 函数返回 tree_dirty=true

  render（shape 后）:
    if tree_dirty:
      restore_scroll_from_anchor  # 用 anchor_N（基于 T_N）+ T_N' → 又一次漂移
      clamp_scroll_top            # sync 到 T_N'
```

实际上 anchor 每帧都被 sync 两遍、scroll_top 每帧被 restore 两遍——只要 tree 在帧中变过，**vertices 已固化但 scroll 已走位**，下一帧用新 scroll 取 doc_line 又把同样的"`length>0 && line_bytes empty`" 窗口拉开。

### S5. 行 553 estimate 与 actual 偏差

estimate（init_display_map 第 242 行）：
```
est_chars_per_line = max(viewport_width / (font_size * 0.6), 40)
est_vl = ceil(byte_length / est_chars_per_line)
```

行 553 长度 69086 B（ASCII），假设 viewport_width = 1200, font_size = 14：
- est_chars_per_line = 1200 / 8.4 = 142.8
- est_vl = ceil(69086 / 142.8) = 484

worker 算出的 actual 用 `compute_visual_lines` 严格按字符级 advance + 词边界，行尾 trim 后仍然≈ 480-490 之间，**差异较小**——但仍然 ≥1，足以触发 `tree_dirty`。

更糟的是 `9ea35cc` 把 worker 限到 max 5000 字节：worker 只 shape 行 553 前 5000 字节，写回的 entry **byte_length = 69086** 但 visual_breaks 只覆盖前 5000 → wrap 数据"残缺"。
渲染主路径在 `render_pipeline.rs:397-431` 的 subset 模式靠 `visual_breaks` 切 byte 范围；但行 553 不属于"is_placeholder && len > 4000"的 placeholder 长行（worker 已写回非空 breaks）→ 走 `!is_placeholder && line_len > max_sync_bytes` subset shape 分支（`render_pipeline.rs:397`）→ 用 `visual_breaks[skip..end_idx]` 切 byte → 但行 553 真实需要 ≈480 visual lines，breaks 只有覆盖前 5000 字节的部分（约 35 个）。`skip` 在屏幕滚到行 553 中段时可能远超 35 → `start_byte = entry.visual_breaks[skip].byte_start` **越界 panic** 或 `end_idx-1` 超过 visual_breaks.len()。

实测 crash.log 的崩溃栈：
```
13  core::panicking::panic_fmt
14  <RangeFrom<usize> as SliceIndex<[T]>>::index (index.rs:569)
17  edit_plus_app::app::App::shape_visible_lines (app.rs:962)  ← shape 阶段切片越界
18  edit_plus_app::app::App::render (app.rs:1140)
```

——和 `9ea35cc` 引入的 max_line_bytes=5000 配合完全一致。该崩溃日期 2026-05-31，是更早的栈，但**同类越界风险仍存在**：worker 写回的 visual_breaks 不再覆盖整行时，渲染端 subset 切片需要做边界保护。

### S6. `update_entry_in_place` 不触发 anchor 重 sync

`drain_reshape_results` 的 `update_entry_in_place` 写完 → 同一函数最后 `rebuild_tree + restore + clamp`。OK。

但 `shape_visible_lines` 循环里的 `update_entry_in_place`（`render_pipeline.rs:824`）**不立即触发 sync**，而是攒着 `tree_dirty_local`，循环末统一 `rebuild_tree`。这个 `rebuild_tree` 在函数内部（`render_pipeline.rs:858`）做：

```rust
if tree_dirty_local {
    display_map.rebuild_tree();
    *tree_dirty = true;
}
```

回到 `render`，`tree_dirty=true` 触发 `restore + clamp`——但**vertices 已经返回到 `shape_verts` 里了**，scroll 调整不会反推 vertices Y。下一帧才用新 scroll 重画。"幽灵带"持续 1 帧但视觉感知 ≥ 60ms。

---

## 四、根因层次（从直接到根本）

```
现象：行号 161 → 164 跳号，中间空带
  │
  ├── 直接原因 A：shape_visible_lines 主循环里某行命中 line_bytes.is_empty() / length=None
  │   → continue 不画行号、不增 counter
  │
  └── 直接原因 B：drain_reshape_results 触发 tree 变 → restore_scroll_from_anchor 用旧 anchor
      → scroll_top 跳到错位置 → display_to_doc(scroll_top.floor()) 命中错的 doc_line
      → 该 doc_line 的 byte_offset 与 LineIndex 暂时不一致（开窗、worker 写回交叉）
      → doc_line_bytes 返回空 → 触发 A
  │
  ├── 间接根本 1：anchor sync 不在 tree 重建之后立即做（撤回的 e580187 是对的）
  │   bbc766b 把 P0-3 撤了，应当恢复
  │
  ├── 间接根本 2：shape 主路径的 `continue` 处不写 placeholder vertex / counter
  │   length=None / line_bytes empty 都该走"渲染行号 + counter+1 + advance placeholder"
  │   这就是 placeholder skip 路径已经做的事情，但被 `length == 0`、`line_bytes.is_empty()`
  │   两个 continue 短路绕过
  │
  ├── 间接根本 3：worker truncate 写回时 visual_breaks 不覆盖整行
  │   subset shape 切片要做严格边界检查；越界时回退到 placeholder skip
  │
  └── 架构性根本：单帧内 tree 变化与 vertices 生成是耦合的
      Zed 用 SnapshotMap + frame snapshot：一帧锁住 tree，
      帧末再交换。这是 P2，但能根治"vertices 与 scroll/tree 不同帧一致"
      所有衍生 bug
```

---

## 五、与之前文档/提交的关系

| 文档 / 提交 | 处理的根因 | 是否解决用户现象 | 备注 |
|---|---|---|---|
| `docs/wrap_pipeline_audit.md`（本次审计） | 列出 16 项 B-bug | 部分（B-2/B-6 相关） | 总览 |
| `e220fac` `7afe60f` (defer scroll) | tree 重建后 vertices 已生成的不一致 | 部分（仍有 anchor 漂移） | 引入 pending_scroll_adjust |
| `e580187` `1f82eed` (sync anchor after rebuild) | pending_scroll_adjust 用旧 anchor | **正确**，仅缺保护层 | 单元测试覆盖了 |
| `b56637a` (placeholder skip 渲行号) | placeholder skip 路径行号缺失 | **不解决短行场景**——max_sync_bytes=4000，crash.log 全部行 < 235 B 不会进这一支 | 修了一个不被触发的路径 |
| `9ea35cc` (max_line_bytes=5000) | worker 单行 shape 时间 | **引入 subset 边界风险**（行 553 actual ≈ 480 vl，worker 只算 5000B 前部） | 与 subset shape 配合需要更强的越界保护 |
| `bbc766b` (revert pending_scroll_adjust) | "理论问题" | **撤错了**——撤回后 anchor 漂移再现 | 撤回时把诊断文档 `ghost_blank_lines_diagnosis.md` 也删了 |

---

## 六、修复优先级

### P0（立即）

1. **shape_visible_lines 主循环的 `continue` 全部补行号 + counter**
   - `render_pipeline.rs:144`：`length == None` → 渲行号、`visual_line_counter += 1`、push advance_cache placeholder。
   - `render_pipeline.rs:334-335`：`line_bytes None / empty` → 同上。
   - 等价于把 placeholder skip 路径里那段（`render_pipeline.rs:354-381`）抽成函数复用。
   - **这条单独就能消除"行号空带"现象**，因为它砍掉了"用户看到现象"的最后一公里。

2. **恢复 `pending_scroll_adjust` + tree-rebuild-后立即 sync_anchor**
   - 重新引入 `e580187` 的 `tree_dirty` 块内 `sync_anchor_from_scroll`。
   - 同时把 `drain_reshape_results` 里的 `restore + clamp` 改成只 `clamp`（内部 sync_anchor），不 restore——避免"用旧 anchor 推 scroll"。
   - 验证：`viewport.rs:842-905` 的两个测试 `sync_anchor_prevents_drift_*` 已经写好。

### P1（架构修补）

3. **subset shape 越界保护**（`render_pipeline.rs:418-426`）：
   ```rust
   if skip < end_idx && skip < entry.visual_breaks.len()
       && end_idx <= entry.visual_breaks.len()
       && (entry.visual_breaks[end_idx-1].byte_end as usize) <= line_bytes.len()
   ```
   越界则**不**走 subset，回退到全行 shape 或 placeholder skip。

4. **worker 写回 truncated entry 时显式标记**：
   - 增加 `DisplayLineEntry.truncated: bool`，渲染端碰到 truncated entry 跳过 subset shape，走 placeholder skip + 渲行号路径。
   - 或者：worker 不写截断的 visual_breaks，写一个特殊的 placeholder（`visual_line_count=est`, `visual_breaks=[(0, byte_length, 0.0)]`）让渲染端看到就 fallback。

5. **统一 `restore_scroll_from_anchor` 调用点**：
   - 当前在 `app.rs` 的 4 处（376/393/416/892/1340/1490）+ `viewport.rs::refold_*` 中各自 `restore + clamp`，时序难追。
   - 统一为：tree 变化 → 仅 `clamp`（含 sync）；只有"逻辑上需要让 viewport 跟随 anchor"（编辑、resize）才 restore。

### P2（架构性）

6. **DisplayLineMap 帧锁**：每帧首固定 tree snapshot，shape 写更新不立即 rebuild_tree，帧末统一交换；如此 vertices 与 tree、scroll 必然同帧一致。Zed 风格 `DisplaySnapshot`。

7. **ResizableLineBytes 抽象**：把 `dv.doc_line_bytes(dl)` 封装为返回 `&[u8]` 而非 `Option<Vec<u8>>`，并保证 LineIndex 与 buffer 在同一时刻原子刷新；杜绝 `length>0 && bytes empty` 窗口。

---

## 七、立刻可用的最小修复（5 行）

把 `render_pipeline.rs` 里两处 `continue` 改为安全占位：

```rust
// L144
let length = if let Some(l) = length { l } else {
    render_line_number_and_advance(/* ... */);
    visual_line_counter += 1;
    advance_cache.push(AdvanceCacheEntry { doc_line: doc_idx, vl_byte_start: 0, clusters: Vec::new() });
    continue;
};

// L334-335
let Some(line_bytes) = line_bytes else {
    render_line_number_and_advance(/* ... */);
    visual_line_counter += 1;
    advance_cache.push(AdvanceCacheEntry { doc_line: range_pre.start + i, vl_byte_start: 0, clusters: Vec::new() });
    continue;
};
if line_bytes.is_empty() {
    /* 同上 */
    continue;
}
```

把 `b56637a` 写在 placeholder skip 那段的行号渲染块抽成 helper（输入 `doc_line_idx, visual_line_counter, ctx, dv, text, gpu`，输出 `Vec<GlyphVertex>`），三处复用。

这条变更**只补"渲染时的兜底"，不动 tree/anchor 逻辑**，回归面最小，应当成为这次的第一刀。

---

## 八、验证方案

### 8.1 单元测试

- `render_pipeline_tests.rs`：构造一个 `dv.doc_line_bytes` 在 `length>0` 时返回 `Some(&[])` 的桩，断言 `shape_visible_lines` 仍画行号、`visual_line_counter` 正确递增。
- `viewport.rs::sync_anchor_prevents_drift_*`：恢复后保留这两个测试。

### 8.2 集成验证

1. 用 crash.log 打开，scroll_top 在行 100-200 反复来回滚动 5 次：行号必须连续，无空带。
2. 滚动到行 500（接近超长行 553），上下 page-up/page-down 5 次：仍然无空带。
3. 在 main 上跑 `cargo test -p edit-plus-app --tests`：通过。
4. **不要** revert `9ea35cc`（worker 5000 字节限制本身有意义），但要补 P1-3 的越界保护。

### 8.3 监控

加一个 cfg(debug) 计数：`shape_visible_lines` 每次走 `length=None` / `line_bytes empty` `continue` 自增并 println；正常滚动应当为 0，>0 即说明 LineIndex/buffer 异步窗口仍存在。

---

## 九、提交建议

```
fix: 幽灵空行——shape 主路径 continue 时补行号占位

之前的修复 (b56637a) 只覆盖了 placeholder skip 路径（max_sync_bytes=4000），
但 crash.log 全部行 < 235B 不进该路径。真正吞掉行号的是 shape 主路径里
length=None / line_bytes empty 的 continue——既不画行号、也不增 counter，
导致行号跳号和空带。

并恢复 e580187 的 tree-rebuild 后立即 sync_anchor，bbc766b 撤回理由错误：
crash.log 短行场景下 placeholder skip 不是触发路径，撤回反而带回 anchor 漂移。
```

— END —
