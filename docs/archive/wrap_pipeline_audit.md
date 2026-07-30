# 换行计算管线 (Wrap Pipeline) 全链路审计

> 调查范围：从 TextBuffer 编辑 → wrap 重算 → DisplayLineMap → 渲染输出的完整路径。
> 调查日期：2026-06-09。
> 主要文件：`crates/core/src/buffer/`、`crates/shaping/src/lib.rs`、`crates/app/src/{app,reshape_worker,display_line_map,viewport,render_pipeline,layout,line_index,cursor_motion,snap_tree,document_view/mod}.rs`。

---

## 0. TL;DR — 八大可恶系统性问题

按严重度排列：

1. **`reshape_worker.rs` 测试编译失败**：`spawn()` 签名加了 `font_family: String`，但测试仍以零参调用（`reshape_worker.rs:318/333/349/366`）。`cargo check -p edit-plus-app --tests` 报 4 处 `E0061`，违反 CLAUDE.md 第 8 条「每次提交可编译」。
2. **DisplayLineMap 在多 tab 下被共享**：整个 App 只有一份 `display_map`、一个 `reshape_worker`、一份 `advance_cache` (`app.rs:140-149`)。切 tab 时仍走旧 wrap 数据；`init_display_map` 只在 open/close 时跑，但 `set_viewport_size` 的 `entries` 也是共享的——已经看到 commit `04272a2` 修了一个相关 stale，但根本设计没改。
3. **三重 wrap 算法不一致**：渲染主路径走 `layout::compute_visual_lines`（带 CJK 边界、空格 trim、prefix sum）；`reshape_worker.process_with_shaper` 走同一函数；但 `reshape_worker.process_fallback` 走完全不同的逐字符 `if line_px + ch_w > viewport_width` 算法（`reshape_worker.rs:236-262`），既不识别词边界、不 trim 空白、也不处理 CJK 标点禁则。一旦 worker 端 shape 失败回退就会让缓存里的 `visual_breaks` 与渲染端不一致。
4. **wrap 宽度读多源**：`render_pipeline.rs:76,198,436` 都用 `ctx.screen_w - 16.0 - ctx.left_margin`；`init_display_map` 在 `app.rs:227` 也复算；`submit_reshape_ahead` 在 `app.rs:1356` 又复算一次。三处必须同步，但 `left_margin` 的来源不一致：渲染时用 `Settings::get().content_left_margin().max(Settings::get().gutter_width(line_count))`，worker 用同一公式但 `line_count` 取自当前 `dv`，而 `init_display_map` 用相同形式——但仍可能在 line_count 跨 tab 切换时漂移。
5. **font_size 缩放后 wrap 缓存不彻底失效**：`app.rs:354-422` 的 ZoomIn/Out/Reset 路径调 `render_cache.invalidate_all()` + `reshape_generation += 1`，但**没**调 `display_map.set_viewport_size(...)` 或主动失效 entries；缓存的 `content_hash` 包含 `font_size_bits`，所以下一帧渲染时会未命中——但 worker 队列里的旧请求只靠 `cancel_before` 拦截，且 worker 端 shaper 的 `font_size` 来自 `req.font_size`（每次 `set_font_size` 都同步），仅有一个隐患：`process_fallback` 用 `req.font_size * 0.6` 估算，没有跟 `pick_char_width` 一致地优先 ASCII 字母数字，导致回退 wrap 偏宽。
6. **DPI 缩放只在初始化生效**：`apply_scale` 只在 `init_window` 调一次（`app.rs:783`）；没有 `ScaleFactorChanged` event handler。把窗口拖到不同 DPI 屏幕上时 `dpi_scale`、`font_size`、`line_height` 全错——继而 wrap 宽度错。
7. **wrap 结果存活时间不一致**：`DisplayLineEntry` 在 `display_map.entries` 里、cluster_data 在 `RenderCache` 里、advance_cache 在 `App.advance_cache` 里，三者按不同生命周期失效（display_map 跟 generation；RenderCache 跟 doc_line range 和 atlas_generation；advance_cache 每帧 drain）。这意味着同一行可能存在三个不一样的 wrap 结果，主要问题是 advance_cache 是 hit-test、cursor_pixel_x 的真实数据源，而它跟 DisplayLineMap 有 race。
8. **viewport 容差与 set_viewport_size 容差不齐**：`display_line_map.rs:89-90` 用 `> 1.0` (width) 与 `> 0.1` (font_size) 容差；但 `wrap_index` 系列文档/计划提到的是 `0.5`；同时 `set_viewport_size` 失效时把所有 `entries.visual_breaks.clear()` + `visual_line_count=1`——这是非常激进的全失效，配合每帧调用会导致大文件 resize 时整个文档重 wrap。

---

## 1. 数据流总览

### 1.1 ASCII 流程图

```
[user types/scrolls/resizes]
     │
     ▼
WindowEvent (winit)                     ── crates/app/src/app.rs window_event
     │
     ├── KeyboardInput → events::handle_keyboard → AppAction::* → dispatch()
     ├── MouseWheel    → handle_scroll → scroll_by_visual_lines → viewport.scroll_by()
     ├── Resized       → pending_resize → flush_pending_resize → resize() → init_display_map()
     ├── Ime::Commit   → execute_edit_command_v2 (×N chars) → display_map.sync(dirty range)
     └── Other         → dispatch / commands

EditCommand path:
     │
     ▼
handle_command(cmd)                     ── crates/app/src/app.rs:~2068
     ├── execute_edit_command_v2(&cmd, dv, &advance_cache)
     │     ├─ commands.rs::execute_edit_command  (mutates TextBuffer, line_index)
     │     └─ returns EditOutcome { dirty_lines, old_line_count, new_line_count }
     │
     ├── if line count changed:
     │     display_map.sync(dirty, placeholder_entries)   ← SnapTree splice, generation += 1
     │     render_cache.invalidate_range(dirty)
     ├── elif dirty_lines: render_cache.invalidate_range
     │
     ├── viewport.restore_scroll_from_anchor(&display_map, line_height)
     ├── viewport.clamp_scroll_top(&display_map, line_height)
     ├── dv.ensure_cursor_visible(&display_map, line_height)
     ▼

Render path (each frame):
     │
     ▼
App::render()                            ── app.rs:1441
     ├── tab_layout = layout_tabs(...)
     ├── self.drain_reshape_results()    ── app.rs:1312    ← merge worker results
     │     ├─ for r in worker.drain_completed(32):
     │     │    if r.generation >= self.reshape_generation:
     │     │       display_map.update_entry_in_place(r.doc_line, r.entry)
     │     │       render_cache.invalidate(r.doc_line)
     │     └─ if any height changed: display_map.rebuild_tree();
     │                                  for dv in workspace: dv.viewport.refold...
     │
     ├── shape_visible_lines(tbh, &mut tree_dirty)        ── render_pipeline.rs:34
     │     ├─ display_map.set_viewport_size(screen_w - 16 - left_margin, font_size)
     │     │    ↳ if width_changed || font_changed: clear all visual_breaks +
     │     │       SnapTree::from_entries(cloned entries) + generation += 1
     │     ├─ start = display_map.display_to_doc(scroll_top.floor() as usize)
     │     ├─ end_display = ceil(scroll_top + viewport_height)
     │     ├─ end = display_map.display_to_doc(end_display - 1) + 1
     │     ├─ skip_visual = scroll_top.floor() - first_doc_display
     │     ├─ for i in 0..vis_count:
     │     │    a) RenderCache hit → emit cached glyph instances + push advance_cache
     │     │    b) miss + line too long + placeholder → defer to async, push empty advance_cache
     │     │    c) shape_fast → fallback to shape → compute_visual_lines or reuse tree breaks
     │     │       → build_advance_cache_entries → render glyph quads
     │     │       → display_map.{sync|update_entry_in_place} + render_cache.insert
     │     │    + cursor_pixel_x via byte_to_x or compute_cursor_pixel_x_cached
     │     │    + visual_line_counter ≥ viewport_height.ceil() + 2 + cursor_done → break
     │     └─ tree_dirty_local → tree_dirty = true
     │
     ├── if tree_dirty: for dv: refold + clamp
     ├── selection_vertices, search highlights, cursor_vertices
     ├── tab_bar / context_menu / search_bar / status_bar / scrollbar vertices
     ├── GPU draw
     ├── post_shape_update()                              ── app.rs:1400
     │     └─ autoscroll: cursor_abs_display = doc_to_display(cursor_doc) + cursor_visual_line_in_doc
     │        if outside [first_vl, first_vl + visible_rows): scroll_to_row + clamp
     └── submit_reshape_ahead()                          ── app.rs:1350
          └─ for dl in start-64..end+64:
                if entry.content_hash != current_hash || is_placeholder:
                   worker.submit(ReshapeRequest { generation, doc_line, ... })

Worker thread (single bg):
     │
     ▼
ReshapeWorker::spawn(font_family)        ── reshape_worker.rs:52
     ├── shaper = Shaper::new()?.with_font_family(&font_family)
     ├── for cmd in cmd_rx:
     │    Shape(req) → if req.generation < latest_generation: skip
     │                 shaper.set_font_size(req.font_size)
     │                 entry = process_with_shaper or process_fallback
     │                 result_tx.send(ReshapeResult { generation, doc_line, entry })
     └── Shutdown: break
```

### 1.2 关键不变量

整个 pipeline 依赖以下不变量保持一致性：

```text
Σ entry.visual_line_count = display_map.tree.total_rows()
                          = scrollbar.total_display_rows
                          = 屏幕滚动条上界

scroll_top ∈ [0, total_rows - viewport_height]  (clamped by clamp_scroll_top)

advance_cache.len() ≈ visible_rows (但有 placeholder 项让索引对齐)

cursor_visual_line ∈ [0, visible_rows) when cursor inside viewport, None otherwise
cursor_visual_line_in_doc = 在所属 doc line 内的视觉行索引（绝对意义，含 skip_visual）

doc_to_display(doc_line) + cursor_visual_line_in_doc = 全局 DisplayRow
```

---

## 2. 各阶段详解（with `path:line` 引用）

### 阶段 A — 文本缓冲（`crates/core/src/buffer/`）

`TextBuffer` 是 gap buffer + history 的标准实现。app 端通过 `DocumentView` 持有 `tb: TextBuffer` + `line_index: LineIndex`。`crates/app/src/line_index.rs:29` 的 `rebuild_from` 在打开文件/编辑后做 byte offset / length 索引；`rescan_from(start)` 是增量重扫的入口。

`LineIndex.lengths[i]` 不含行结束符（`line_index.rs:51`），但 `byte_offset/byte_length` 在喂给 worker 时（`app.rs:1383-1395`）包括下次行起前的全部字节——CRLF 处理在 `rebuild_from` 里把 `\r\n` 当一对处理。

**问题 A1 — 行长度的语义不一致**
* `line_index.lengths[i]` 不含换行符。
* `display_map::DisplayLineEntry.byte_length` 在 worker 路径填的是 `bytes.len()`（`reshape_worker.rs:153,225`），在主线程 `render_pipeline.rs:798` 填 `line_bytes.len() as u32`。`line_bytes` 来自 `dv.doc_line_bytes(dl)`——需要确认这个 API 是否包含 trailing newline。如果两侧定义不一致，`content_hash` 就会按不同 length 计算，导致 worker 写入的 entry 永远命中不了主线程的 hash 比较，每帧重新提交。
* 排查命令：`grep -n "doc_line_bytes\|line_byte_length" crates/app/src/document_view/mod.rs`。

### 阶段 B — Shaping（`crates/shaping/src/lib.rs`）

`Shaper` 包装 cosmic-text + ttf-parser + swash。

* `shape(text)` (`lib.rs:183`)：完整 OpenType pipeline，处理 RTL/合字/复杂脚本。
* `shape_fast(text)` (`lib.rs:221`)：用 ttf-parser 直接 `glyph_index + glyph_hor_advance`。**任意一个字符的 glyph_index 缺失都直接返回 Err**（`lib.rs:298-301`）→ 这意味着 emoji 或不在主字体里的 CJK 都会 fall back 到完整 `shape`。
* `font_size` 用 `Metrics::new(font_size, line_height)` 设置（`lib.rs:184`）；`set_font_size` 只更新内部字段，不重置 buffer——但 `shape` 入口里 `set_metrics` 重新生效（`lib.rs:192`）。

**问题 B1 — `shape` 与 `shape_fast` 的字体可能不同**
`shape` 用 `Family::Name(font_family)` 或 `Family::Monospace`（`lib.rs:185-188`）；`shape_fast` 优先 `Family::SansSerif`（`lib.rs:230`）→ Monospace → Serif（`lib.rs:233-241`）。所以同一行 fast 与 slow 路径选的字体可能不一样，advance 不一样，wrap 结果不一致。这是 `process_with_shaper` 在 `reshape_worker.rs:159-165` 用 fast → fallback 到 shape 的隐患：缓存里的 `visual_breaks` 是 SansSerif 算出来的，渲染主路径用同一逻辑——但如果主路径走 shape（因为 fast 失败），SansSerif 就被换成 Monospace；主路径用同一 char_width，但行宽 prefix 已经按不同 advance 计算。

**问题 B2 — `Shaper::new` 的 line_height 与 Settings 的 line_height 公式不同**
`Shaper::new()` 把 `line_height = font_size * 1.4`（`shaping/lib.rs:139`）。
`Settings::set_font_size` 把 `line_height = font_size * 1.618`（`settings.rs:55`）。
两者不在同一计算空间。Shaper 内部的 line_height 仅影响 cosmic-text 的 buffer metrics，不直接影响 wrap，但这种数值不一致是潜在 confusion 源。

**问题 B3 — `GraphemeAdvanceCache` 的 LRU 容量与缓存键**
`MAX_CACHE_SIZE = 4096`（`shaping/lib.rs:57`），key = `(grapheme, font_size_quantized, attrs_hash)`（`lib.rs:74`）。**没有 viewport_width 维度**——这是合理的（advance 只跟字体/字号有关）。但调用 `with_font_family` 后 `attrs_hash` 已变（`lib.rs:163`），缓存被旁路；调用 `set_font_size` 不变 attrs_hash 但变 font_size_bits 量化后值——也旁路。这其实是好的失效行为。**但**：在 `set_font_family` 后没有清空缓存，旧条目占用容量直到 LRU 自然驱逐。这会浪费内存而非引发正确性问题。

### 阶段 C — DisplayLineMap / SnapTree（`crates/app/src/display_line_map.rs` + `snap_tree.rs`）

`DisplayLineMap` 同时持有：
* `tree: SnapTree`（B-tree，O(log n) 查询）
* `entries: Vec<DisplayLineEntry>`（O(1) 索引）
* `viewport_width: f32, font_size: f32`（用于 set_viewport_size 失效判断）
* `generation: u64`

`SnapTree::splice` 是简化实现（`snap_tree.rs:161-188`）：把所有 entries 全 collect 出来 → splice → `Self::from_entries(entries)`，**O(N)**。基准测试显示 18000 entries × 16 splice 大概 16ms（`snap_tree.rs:387-434`），按 60fps 已显著占帧。一次编辑通常只 1 splice，每秒最多 60 次 splice，能过；但拖拽 IME 候选连续输入时会把 splice 频次推高。

**问题 C1 — `set_viewport_size` 是核弹失效**
`display_line_map.rs:88-107`：

```rust
pub fn set_viewport_size(&mut self, width: f32, font_size: f32) {
    let width_changed = (self.viewport_width - width).abs() > 1.0;
    let font_changed = (self.font_size - font_size).abs() > 0.1;
    self.viewport_width = width;
    self.font_size = font_size;
    if width_changed || font_changed {
        for entry in &mut self.entries {
            entry.visual_breaks.clear();
            entry.visual_line_count = 1;
        }
        self.tree = SnapTree::from_entries(self.entries.clone());
        self.generation += 1;
    }
}
```

* 整个文档（哪怕 100k 行）一次循环 + `entries.clone()` + 全树重建。大文件 resize 一次掉帧严重。
* `entries.clone()` 会克隆 `SmallVec<[VisualBreak;1]>` 共 16+ bytes per row × 100k → 1.6MB 拷贝。
* 调用方是 `render_pipeline.rs:76`，**每帧首先调用一次**。即使 width/font 没变，函数内部仍会 `viewport_width = width; font_size = font_size`——OK，但容差判断 `> 1.0` / `> 0.1` 可被亚像素抖动绕开（实际 width 通常是整数 GPU pixels，应该没问题）。

**问题 C2 — `DisplayLineMap` 是 App 级单例，不区分 tab**
`App.display_map` 是单一实例（`app.rs:145`）。每个 `DocumentView` 自己有 `viewport`，但 wrap 数据只有一份。`init_display_map(dv_idx)` 用 `dv_idx` 索引活跃 dv（`app.rs:224-244`），但同一份 `display_map` 在多个 dv 间共享。一个 dv 切到另一个 dv 时（`workspace::switch_to`）：
* 旧 dv 的 wrap entries 被丢弃（`init_display_map` 全部覆写）。
* `reshape_worker` 队列里可能仍有上一个 dv 的请求；下一帧 `drain_reshape_results` 会把它们写进**新 dv 的** display_map（因为 worker 不知道是哪个 dv）！
* `r.doc_line` 是按上一个 dv 的索引，新 dv 的 line_count 可能不一样 → 把错误的 entry 塞进新 dv。
* 缓解：`generation` 每次 init_display_map 不递增——只在 set_viewport_size 和 sync 时递增。所以 stale 请求不会被 generation 过滤掉。这是个**正确性 bug**。
* 我已确认 `app.rs:875` 在 zoom 时递增 generation，但 tab switch 路径——commit `04272a2` 注释提到了「viewport stale」并加了 reshape_generation += 1，让我再核对…

```
$ grep -n "init_display_map" crates/app/src/app.rs
194:                    self.init_display_map(self.workspace.active_index);
827:                self.init_display_map(self.workspace.doc_views.len() - 1);
873:                self.init_display_map(self.workspace.active_index);
```

`app.rs:189-192`（WorkspaceEffect::TabSwitched 路径）和 `app.rs:870-878`（resize width_changed 路径）都递增了 generation。但 `app.rs:827`（open_file 添加 tab）路径**没有**递增 generation，**也没** cancel_before。考虑到：第一次 open_file 后 worker 队列是空的，问题暂时不会触发；但如果 open_file 在已有 tab 状态下被调用（用户快速打开多个文件），上一个 tab 的请求可能仍在队列里，会被错误归到新 tab 的 display_map。

### 阶段 D — Wrap 算法（`crates/app/src/layout.rs::compute_visual_lines`）

`layout.rs:158-301` 是核心 wrap。基于前缀和 `prefix[i]`（`layout.rs:170-179`）+ 单循环。维护：
* `last_break_after_ws`：最后一个空白后第一个非空白簇的位置。CJK 模式下不记录（`layout.rs:206-210`），强迫 CJK 段填满行。
* `last_break_cjk`：最后一个 CJK/non-CJK 边界。
* `last_content_cjk: Option<bool>`：当前段最后一个有内容簇是否 CJK。

**Break 选择**（`layout.rs:240-271`）：
1. 默认 hard break 在 ci 处，宽度 = `width_of(start, ci)`。
2. 如果有 `cand_ws`，候选「按词断」处的 trim_width。CJK 模式：要求 `ws_x >= best_x`；非 CJK：要求 `ws_x >= hard_x * 0.5`（避免短词抢断）。
3. 如果有 `cand_cjk`，且断点不是孤立标点（`is_punctuation`），候选 CJK 边界。
4. 选其中**最宽**的（`>= best_x`）。

**CJK 空格回收**（`layout.rs:222-233`）：
当前簇是空白且上一个内容簇是 CJK 时，向前 peek 找到下一个非空簇，如果「现行宽度 + 空格 + 下一簇 ≤ viewport_width」则跳过空格继续 packing。这是中文 + 空格场景的特殊处理，避免 ASCII 空格触发提前断行。

**Continuation trim**（`layout.rs:279-282`）：断点设定后，跳过续行行首所有空白。这个跟 `process_fallback` 不一致（fallback 不 trim）。

**问题 D1 — `compute_visual_lines` 在 reshape worker 与渲染主路径返回 Vec<(usize,usize,f32)>**
返回类型是「cluster index 范围」，但 worker 把它转为 `VisualBreak { byte_start, byte_end, pixel_width }`（`reshape_worker.rs:172-177`）。从 cluster 范围转字节范围通过 `shaped.clusters[start].byte_range.start` / `clusters[end-1].byte_range.end`——OK 但前提是 `shape_fast`/`shape` 输出的 clusters 顺序与字节顺序一致。RTL 文本（cosmic-text 的 layout_runs）可能不按字节顺序输出！这会让 `byte_start > byte_end` 反转，下游 `render_pipeline.rs:440-441` 用 partition_point 找 cluster 起止时假设 byte_range 单调，会出错。
* 例：阿拉伯文 + 英文混排。
* 缓解：`shape_fast` 不支持 RTL（无 BiDi），所以走 `shape` 路径才会触发；`shape_arabic_rtl_doesnt_crash` 测试只检查不 crash 不验证 wrap 正确性。

**问题 D2 — 极窄视口的死循环风险**
`layout.rs:236` 条件 `visual_line_x + adv[ci] > viewport_width && ci > start`：当 `ci == start` 且单簇宽度大于 vp 时，**不进入 break 逻辑**，继续往下走 `ci += 1`。下一次 ci > start 才会处理，但此时 visual_line_x 已经超过 vp 一大截。结果：极窄视口（vp < 单字符宽度）会得到一行只有一个簇（OK）但宽度记录 = `width_of(start, n)` 远超 vp。这跟 `plans_wrap_algo_fix.md` Task 8 说要做的「极窄视口强制断行 + 续行 trim 行首空白」对应。计划标了 [x]——但代码里没看到 `must_break && ci == start` 的分支！计划可能没真正实现这一部分。

**问题 D3 — `last_break_cjk` 的 reset 时机**
break 选择后（`layout.rs:284-291`）三个候选都 reset，并重新初始化 `last_content_cjk`。但 `last_break_after_ws = None` 之后，如果新行起始 = 前一段尾部 CJK，下一次遇到空格→非 CJK 就会重新设置 `last_break_after_ws`。这是对的。
**但**：如果新行刚好是「续行第一字符 = 空白」，trim 后 `start = 跳过的位置`，此时 `ci` 跟 `start` 同步推进，但旧的 `last_content_cjk = None` 在循环顶部被重新设置（`layout.rs:289-291`）；若续行第一个非空字符是 ASCII，`last_content_cjk = Some(false)`——这是对的。
但 `last_break_after_ws = None` 不会立刻重置：循环顶部只检查「ci > 0 && ws_arr[ci-1]」。如果续行起始 `start` 跳过了空白，那么 `ci` 也被同步到 start（`layout.rs:288`），「ci-1」可能仍然是空白或更早的非空白。这是个已知 bug 来源——但实际复现要复杂边界场景。

**问题 D4 — `process_fallback` 与 `compute_visual_lines` 的根本算法分歧**
`reshape_worker.rs:236-262`：

```rust
for (ci, ch) in line_str.char_indices() {
    let ch_w = if is_cjk_char(ch) { cjk_w } else { ascii_w };
    if line_px + ch_w > viewport_width && ci > byte_pos {
        breaks.push(VisualBreak { byte_start, byte_end: ci, ... });
        byte_pos = ci;
        line_px = ch_w;
    } else {
        line_px += ch_w;
    }
}
```

* **没有词边界检测**：连续 ASCII 单词在最后一个字母处硬断，不在空格处断。
* **没有 CJK 边界检测**：CJK + ASCII 混排时不会优先在边界断。
* **没有空白 trim**：续行可能以空白开头。
* **没有 punctuation 禁则**：CJK 标点可能孤立在行首。

这个 fallback 仅在 `Shaper::new()` 失败时触发（`reshape_worker.rs:62-64`）——即字体加载失败的极端场景。但代码里它也作为 `process_with_shaper` 内 `shape` 失败的兜底（`reshape_worker.rs:163`）。同一个 doc line 在不同时刻可能命中不同 wrap 算法 → cache 数据漂移。

### 阶段 E — Reshape Worker（`crates/app/src/reshape_worker.rs`）

* 单线程、单 worker。
* `cancel_before(generation)` 用 `latest_generation` 原子变量，只检查请求自身的 generation 和 worker 当前 latest（`reshape_worker.rs:69-71`）。
* 主线程 `drain_reshape_results` 也做一次 `r.generation >= self.reshape_generation` 检查（`app.rs:1319`）。

**问题 E1 — Worker shaper 不知道 viewport_width 之外的 wrap context**
worker 接收 `font_size`、`viewport_width`，但**不**接收：
* `font_family`：每次 `set_font_size` 调用 shaper，但 shaper 在 spawn 时已 `with_font_family(font_family)` 锁定。如果用户在运行时换字体（`Settings::set_font_family`），主线程的 shaper 切了字体但 worker 没切。当前没有 UI 让用户换字体，所以暂未触发；但代码没保护这条路径。
* `tab_width`：`process_fallback` 没读 `tab_width`，硬编码 `\n` 单独成段；TAB 字符走 `is_cjk_char(ch) → false → ascii_w`，**不**乘 4。这跟 `layout.rs::ws_cluster_advance` 不一致。
* `dpi_scale`：worker 不知道 dpi_scale，但 viewport_width 已经是物理像素，所以 OK——除非 dpi 变了主线程没传新值。

**问题 E2 — Generation 不区分 dv**
`reshape_generation` 是 App 级单调递增。`ReshapeRequest` 没带 dv_index 或 doc_view 标识。worker 队列只能在 generation 维度过滤过期请求；切 tab 时旧 dv 的请求虽然 generation 也 += 1（4 处都做了 cancel_before），但**已经发出**的请求如果在 worker 处理一半，结果会带 generation = 旧值发回来，主线程 `r.generation >= self.reshape_generation` 严格 ≥，所以会被拒绝。OK——这里写得对。
* 但「已经处理一半」是基于 worker 的 `latest_generation.load()`：worker 在循环顶部 `if req.generation < worker_generation: continue` 跳过过期请求，已经在跑的不会中途打断。一个长行 shape 一次约几 ms，期间 generation 升级可能浪费 worker 时间。

**问题 E3 — drain_reshape_results 的 height_changed 触发 rebuild_tree**
`app.rs:1320-1338`：检查 `old.visual_line_count != r.entry.visual_line_count`，如果 changed 标记 tree_dirty 并最后 `rebuild_tree`。这是 O(N) 全树重建。对于一次 worker 提交 32 个 results 的 batch，最多 1 次 rebuild_tree——OK。
* 但 `update_entry_in_place` 不动 tree（`display_line_map.rs:184`）；这意味着如果 32 个 entry 中 31 个 height_changed，只 rebuild 1 次（合理）。如果 0 个 height_changed，update_entry_in_place 不更新 tree，tree 的 `total_rows` 滞后于 entries——但 `total_rows` 此时是对的（行数没变），只有 `visual_breaks` 内容更新。OK。

### 阶段 F — Render Cache（`crates/app/src/render_cache.rs`）

存储 per-doc-line 的 `CachedLine`：glyph instances、cluster_data（byte_range + advance）、visual_lines 范围、subset_start。
* `subset_start` 字段（`render_pipeline.rs:818`）记录这个缓存对应的 wrap 子集起点。允许针对长行只缓存「从某 visual_line 开始」的部分。
* `is_full_line || is_perfect_subset`（`render_pipeline.rs:213-216`）：判断当前请求能否复用缓存。
* `content_hash` = `(byte_offset, byte_length, viewport_width_bits, font_size_bits)` 的 wrapping_mul（`render_pipeline.rs:199-206`）。**不含字节内容**——同一 (offset, length) 但不同字节内容会击穿缓存命中：哈希同 → 错误命中！

**问题 F1 — content_hash 不含字节内容**
这个跟 `plans_wrap_algo_fix.md` Task 3 处理过的 shape_cache key 问题是同一类。修复方案是 cache key 含 (offset, length) + 编辑后用 `invalidate_line` 清。当前主线程编辑路径（`app.rs:2090-2093`）调 `render_cache.invalidate_range(dirty.clone())`——OK，编辑会失效。但**外部修改**（如文件被外部进程改动后重新加载）走的是 `init_display_map` 路径，会把 entries 整体替换，但 `render_cache` 没失效！会渲染旧字节。

### 阶段 G — Viewport（`crates/app/src/viewport.rs`）

* `scroll_top: f64`（DisplayRow 单位，含小数部分）
* `visible_rows: usize`（屏幕容纳的视觉行数）
* `viewport_height: f64`（屏幕高度 / line_height，可能是分数）
* `scroll_anchor: ScrollAnchor { doc_line, pixel_offset }`（编辑稳定锚点）

`visible_doc_line_range(map)` 通过 display_map 精确映射（`viewport.rs:168-177`）；`visible_doc_line_range_approx` 跳过 wrap 知识（`viewport.rs:181-187`）——后者据 `plan_wrap_algo_fix.md` Task 10 应该被 deprecated，但目前还在用。

**问题 G1 — `clamp_scroll_top` 与 `clamp_scroll_top_no_wrap` 双轨**
`viewport.rs:202-218`：第一个用 `map.total_rows()`；第二个用 `total_lines`。后者在没有 wrap 知识的情形（init/resize 早期）才该用。问题是：`resize` 路径（`app.rs:886-892`）调 `restore_scroll_from_anchor + clamp_scroll_top`——前者把 scroll_top 设到 anchor 推导值，后者按 display_map.total_rows clamp。但此时 display_map 可能还是 stale（width 已经变了，set_viewport_size 还没在下一帧的 shape_visible_lines 调用）→ clamp 边界是「旧 width 下的 total_rows」，而 scroll_top 期望按新 width 的 wrap。结果可能是 scroll_top 短暂过大。下一帧 shape_visible_lines 进入会 set_viewport_size → invalidate all → total_rows = entries.len()（每行 visual_line_count = 1）→ clamp 把 scroll_top 拉回。视觉上是一帧抖动。

**问题 G2 — `restore_scroll_from_anchor` 用整数 `display_row` 反推**
`viewport.rs:236-241`：

```rust
pub fn restore_scroll_from_anchor(&mut self, map, line_height) {
    let display_row = map.doc_to_display(self.scroll_anchor.doc_line) as f64;
    let lh = line_height.max(1.0) as f64;
    self.scroll_top = display_row + self.scroll_anchor.pixel_offset as f64 / lh;
}
```

`pixel_offset` 是「anchor doc_line 起始位置往下偏 N 像素」。问题：anchor.doc_line 在 wrap 后可能跨多个 visual line（即对应 display_row.. display_row + N），但 anchor 只记录起始 display_row。`pixel_offset / line_height` 给出的是「anchor doc_line 内偏多少 visual line」——但 wrap 后 line_height 没变，所以这部分对——但 `pixel_offset` 在 `sync_anchor_from_scroll` 是 `sub_row * line_height`（`viewport.rs:233`），其中 `sub_row = scroll_top.fract()`。这是「sub-line 偏移」，不是「anchor doc_line 内的 visual line 偏移」。
* 如果 anchor 记录时 `scroll_top = 5.7`、对应 doc_line = 3、doc_to_display(3) = 5：`sub_row = 0.7`、`pixel_offset = 0.7 * line_height`。
* restore 时：`display_row = doc_to_display(3) = 5`（**假设 anchor 之前的行没改变** wrap）→ `scroll_top = 5 + 0.7 = 5.7`。OK。
* **但**如果 doc 0..2 的 wrap 在编辑后膨胀（如 doc_line 1 从 1 visual line 变 5），现在 `doc_to_display(3) = 9` → `scroll_top = 9 + 0.7 = 9.7`。视口跟着 doc_line 3 的内容向下移动到第 10 行——**这是符合期望的**：scroll 锚定 doc_line 而非物理 row。
* 但「doc_line 3 自己 wrap 变了」呢？比如 anchor 时 doc_line 3 是单行；编辑后 doc_line 3 变 5 visual line。restore 时 anchor.doc_line 仍是 3、pixel_offset 仍是 0.7\*lh。`doc_to_display(3)` = 9，但「anchor 时其实 viewport 显示的是 doc_line 3 的第 0 visual line」。restore 后还是落在 doc_line 3 的第 0 visual line 上方 0.7\*lh——OK。
* 真正的问题是：anchor 记录时 viewport 可能在「doc_line 3 的第 2 visual line」，sub_row = 0.7，pixel_offset = 0.7\*lh。但 anchor.doc_line = 3、不记 visual_line_in_doc。restore 时 `scroll_top = doc_to_display(3) + 0.7 = 9.7`——落到 doc_line 3 第 0 visual line + 0.7\*lh，跳过了原来的 2 visual line！**这是 anchor 模型的根本缺陷**。

### 阶段 H — Cursor / Move / Hit Test（`crates/app/src/cursor_motion.rs`）

* `move_cursor_visual` 三分支（4a/4b/4c）。
* `find_visual_line_index` 用 bounds 二分（`cursor_motion.rs:85-98`）。
* `find_closest_offset` 用 sticky_x 找最近列。`cum_x = 32.0 * dpi_scale` 起步——硬编码 `32.0`，应该用 `Settings::content_left_margin()`。**问题 H1**：dpi_scale 已经包含但 left_margin 应该取自 Settings 或 ctx，否则 gutter_width 大于 32px 时（line_count > 999）列对齐会错。

`render_pipeline::shape_visible_lines` 里 cursor 像素位置由 `byte_to_x`（`render_pipeline.rs:554`）+ `compute_cursor_pixel_x_cached`（`render_pipeline.rs:303-310`）算出。两条路径必须输出一致——计划 Task 9 中 `pick_char_width` 改为优先 ASCII，但只影响 wrap，不影响 cursor x。

**问题 H2 — cursor_visual_line 在 cache hit 路径用 `display_map.doc_to_display`，在 miss 路径用同一函数**（`render_pipeline.rs:293,532`）
都对，但 cache hit 路径用 `cached.subset_start` 加偏移（`render_pipeline.rs:291`），miss 路径用 `if shape_subset_only { cursor_vl_in_doc_all + actual_skip } else { cursor_vl_in_doc_all }`（`render_pipeline.rs:531`）。两条路径的 `cursor_vl_in_doc_all` 来源也不同（cache：在 cached.visual_lines 中查找；miss：用 `find_visual_line_index(bounds, cursor_col)`）。**两条路径很可能在 subset_start > 0 的边界条件下不一致**。

### 阶段 I — Autoscroll（`crates/app/src/app.rs::post_shape_update`）

`app.rs:1400-1431`：

```rust
let cursor_abs_display = self.display_map.doc_to_display(cursor_doc_line)
                       + dv.cursor_render_state.cursor_visual_line_in_doc;
let first_vl = dv.viewport.first_visible_row();
let visible_rows = dv.viewport.visible_rows;
let last_vl = first_vl + visible_rows as u32;
if cursor_abs_display < first_vl.as_usize() {
    dv.viewport.scroll_to_row(cursor_abs_display as f64);
    dv.viewport.clamp_scroll_top(...);
} else if cursor_abs_display >= last_vl.as_usize() {
    let target = (cursor_abs_display as f64) - (visible_rows as f64) + 1.0;
    dv.viewport.scroll_to_row(target.max(0.0));
    dv.viewport.clamp_scroll_top(...);
}
```

**问题 I1 — autoscroll 在 render() 之后调用，下一帧才生效**
`render()` 顺序（`app.rs:1441-1710`）：drain_reshape_results → shape_visible_lines（写 cursor_visual_line_in_doc） → ... → GPU draw → post_shape_update（autoscroll）。意味着：
1. 帧 N：cursor 移动 → handle_command → ensure_cursor_visible（doc-line 粒度，可能不准）。
2. 帧 N：render 开始 → shape 把 cursor_visual_line_in_doc 写好 → draw 完毕（这一帧光标可能在屏幕外）→ post_shape_update 把 viewport 调对。
3. 帧 N+1：render 用新 viewport，光标进屏。

用户看到的是「按下方向键，第一帧光标消失/在边缘，第二帧才到中间」。这跟 `displayrow.md` § 5.2 「Phase 5: ensure_cursor_visible 移除」的预期一致——但 ensure_cursor_visible 实际还在（`app.rs:911,1434(注释)` 等多处）。**两个 autoscroll 入口未合并**，`displayrow_review.md` Bug 3 已经标记。

**问题 I2 — `cursor_visual_line_in_doc` 来自 shape，但 shape 用的是当前帧的 display_map**
shape 时调 set_viewport_size，可能整体失效；如果 cursor 所在行此帧没被 shape（命中 cache），那么 cached.subset_start 来源于上次 shape——但 viewport_width 上次就该是相同值（cache 用 viewport_width 做哈希）。OK。
但 long line 的 async 路径（`render_pipeline.rs:349-364`）在 placeholder 状态下完全跳过 shape，`cursor_visual_line_in_doc` 没有被更新——**autoscroll 用的是上一帧的旧值**，可能滚到错误位置。

---

## 3. 跨切关注点

### 3.1 DPI / Scale Factor

* `Settings::apply_scale` 只在 `init_window` 调用一次（`app.rs:783`）。
* `WindowEvent::ScaleFactorChanged` 没有 handler——拖窗到不同 DPI 屏不会更新 dpi_scale。
* `gpu.ctx.config.{width,height}` 是物理像素；`Settings::dpi_scale` 是对 font_size、line_height、status_bar_height 的乘子。`Settings::content_left_margin()` 也乘 dpi_scale。
* viewport_width = screen_w(物理) - 16 - left_margin(物理)：物理像素维度自洽。
* **但**：`16.0` 是硬编码（`render_pipeline.rs:76,198,436` + `app.rs:227,1356`），不乘 dpi_scale。**这是 bug**：在 Retina 屏上 16 物理像素 ≈ 8 logical pt，看起来 scrollbar 留白只一半。计划 `docs/plans_dpi_scale_refactor.md` 应该处理过——但代码没改。

### 3.2 Worker / 主线程同步与竞态

| 触发 | reshape_generation | render_cache | display_map | worker queue |
|---|---|---|---|---|
| 编辑 (handle_command)  | 不递增 | invalidate_range(dirty) | sync(dirty, placeholders) | **未 cancel** |
| IME commit | 不递增 | invalidate_range | sync | **未 cancel** |
| Zoom In/Out/Reset | += 1 + cancel_before | invalidate_all | **未 set_viewport_size** | cancel |
| Width changed (resize) | += 1 + cancel_before | invalidate_all | init_display_map (重置 entries) | cancel |
| Tab switched | += 1 + cancel_before | invalidate_all | init_display_map | cancel |
| Open file | **不递增** | (no explicit) | init_display_map | **未 cancel** |
| set_viewport_size (in shape) | 不递增 | (no explicit) | clear visual_breaks + rebuild_tree | 不影响 |

**问题 3.2.1 — 编辑路径不 cancel worker 也不递增 generation**
当用户在 line 100（屏幕外）打字、worker 有 line 100 的请求在排队，结果是：
1. 主线程编辑 → display_map.sync（line 100 替换为 placeholder）。
2. worker 算完旧字节的 wrap → drain_reshape_results 检查 `r.generation >= self.reshape_generation`——generation 没变，**通过**！
3. 旧 wrap 数据写入新 entry → 渲染错误。
* 缓解：`render_pipeline.rs:447,376` 在拿 entry 时会比 `entry.content_hash == content_hash_fast`，hash 不一致就走重新 shape——OK 这层防护住了。但 placeholder 一直被 worker 旧数据覆盖，就一直撞 hash 不一致，反复 shape 浪费 CPU。
* 修复：编辑后递增 reshape_generation。

**问题 3.2.2 — RTL / 复杂脚本场景下 worker shape_fast 永远失败**
`shape_fast` 检测到任何缺失 glyph 即 fail（`shaping/lib.rs:298-301`），落到 `process_with_shaper` 内 `shape` 路径。这条路径用 `Family::Name(font_family)` 而非 SansSerif → CJK / emoji 用主字体可能不全 → 又触发 fail → fallback 到 `process_fallback` 字节估算。**fallback 算法跟主路径不一致**（问题 D4）→ 进入 display_map 的 entry 跟主路径渲染时 `compute_visual_lines` 不一致 → cursor 列错位、滚动条总行数错。

### 3.3 缓存生命周期与失效完整性

| 缓存层 | 失效触发 | 生命周期 | 漏掉的失效 |
|---|---|---|---|
| `shape_cache` (Shaper) | LRU 4096 | 长期 | font_family 变更（attrs_hash 变 → 自动旁路 OK） |
| `display_map.entries[].visual_breaks` | width/font 变 + 主线程 sync | 长期 | 字体/语言 attrs 变化、tab_width 变化 |
| `render_cache.cache[doc_line]` | invalidate_range / invalidate_all | 跨帧 | 外部文件重载、theme 变（颜色变了，但 cache 里 highlight_kind 是 enum 数值，theme 变只改色板——OK） |
| `advance_cache` | 每帧 drain | 单帧 | — |
| `first_line` / `last_line` (LineCache) | 每帧覆写（但只在 shape_subset_only=false 时） | 单帧 | shape_subset_only=true 时不更新——长行场景 first_line 可能滞后 |

**问题 3.3.1 — Theme 变 / 语法高亮变不失效 RenderCache**
`CachedLine` 里存的是 `highlight_kind: u8`（enum 数值）+ atlas slot。颜色映射在渲染时用 `theme.foreground` 等。所以 theme 变化不需失效 cache——OK。但如果**语法高亮重算**了（如 LSP 异步返回），highlight spans 变了，`render_cache` 里的 highlight_kind 是旧的，必须失效。当前代码我没找到触发点——LSP 集成可能还没完成（`crates/lsh/`），但留 TODO。

**问题 3.3.2 — `first_line`/`last_line` 在 shape_subset_only 下不更新**
`render_pipeline.rs:478-495`：subset 模式不写两个 LineCache。`move_cursor_visual` 4b/4c 用它们处理「光标超出屏幕的方向键」（`cursor_motion.rs:155,213`）。subset 模式触发条件是 `is_placeholder=false && line_len > max_sync_bytes && entry 有 valid breaks`——也就是「长行有缓存」。这种情况下用户按方向键超出屏幕时，4b/4c 拿到的是其它行的 first_line/last_line，定位完全错。

### 3.4 性能总览

* `set_viewport_size` 每帧调用，但只在 width/font 变时做 O(N) 全失效——OK。
* `display_map.sync` 编辑路径每次 splice 是 O(N)（snap_tree.rs:163 collect → splice → from_entries），大文件每键 1ms+。
* `compute_visual_lines` 已经是 O(N) prefix sum——OK。
* worker 单线程：长行 shape 几 ms，user 同时排队多个请求会串行化。
* `init_display_map` 创建 N 个 placeholder entries（O(N)），但只在 open/tab switch/width change 触发——OK。

### 3.5 已知正确性 bugs（汇总）

按可触发概率：

| 编号 | 描述 | 触发条件 | 严重度 |
|---|---|---|---|
| **B-1** | reshape_worker 测试编译失败 | `cargo test -p edit-plus-app` | 中（CI 红） |
| **B-2** | 编辑后 worker 旧请求覆盖 placeholder | 屏幕外行编辑 + worker 在跑 | 高（短暂错位） |
| **B-3** | `process_fallback` 与主路径 wrap 算法不一致 | shape 失败（罕见 / RTL） | 中 |
| **B-4** | DPI 变化不重算 | 多屏拖窗 | 中 |
| **B-5** | autoscroll 延迟一帧 | 任何 cursor 移动 | 低（视觉抖动） |
| **B-6** | 长行 subset 模式 first_line/last_line 不更新 | 长行 + 方向键超出屏幕 | 高（光标乱跳） |
| **B-7** | `shape_fast` SansSerif vs `shape` Monospace | 缺 glyph 字符 | 中 |
| **B-8** | 16.0 硬编码不乘 dpi_scale | Retina 屏 | 低（视觉） |
| **B-9** | open_file 添加 tab 路径不 cancel worker | 快速连开多文件 | 低 |
| **B-10** | shape_subset_only 路径 cursor_vl_in_doc_all 计算分歧 | 长行命中 cache 边界 | 中 |
| **B-11** | RTL clusters 反序 byte_range 单调假设破坏 | 阿拉伯/希伯来文混排 | 中 |
| **B-12** | render_cache 不在外部文件重载时失效 | 外部进程改文件 | 低 |
| **B-13** | scroll_anchor.pixel_offset 不记 visual_line_in_doc | 锚定行内多 visual line | 中 |
| **B-14** | DisplayLineMap 单例 + worker 共用，结果可能跨 dv | 切 tab 与 worker 处理重叠 | 中 |
| **B-15** | 极窄视口 (vp < 单字符宽度) 不强制断 | 窗口拖到极窄 | 低 |
| **B-16** | settings 与 shaper 的 line_height 公式不同 (1.618 vs 1.4) | 仅当主线程读 shaper.line_height 时 | 低（目前没读） |

---

## 4. 关键代码位置速查

### 主要决策点

```
WRAP WIDTH 计算:
  render_pipeline.rs:76           display_map.set_viewport_size(screen_w-16-left_margin, font_size)
  render_pipeline.rs:198,436      let viewport_width = ctx.screen_w - 16.0 - ctx.left_margin
  app.rs:227 (init_display_map)   screen_w - 16.0 - left_margin
  app.rs:1356 (submit_reshape)    screen_w - 16.0 - left_margin
  layout.rs:158 (compute_visual_lines)  takes viewport_width param

WRAP 算法:
  layout.rs:158-301               compute_visual_lines (主路径 + worker shaper 路径)
  reshape_worker.rs:236-262       process_fallback (shaper 失败时的兜底)
  layout.rs:91-102                cluster_boundary_class
  layout.rs:66-84                 is_cjk_char
  layout.rs:41-47                 is_whitespace_cluster
  layout.rs:54-63                 ws_cluster_advance (TAB = char_width × 4)
  layout.rs:112-135               pick_char_width (优先 ASCII alphanumeric)

WRAP 结果存储:
  display_line_map.rs:33          DisplayLineMap { tree, entries, viewport_width, font_size, generation }
  snap_tree.rs:13-21              DisplayLineEntry { visual_line_count, visual_breaks, byte_offset, byte_length, content_hash }
  snap_tree.rs:23-28              VisualBreak { byte_start, byte_end, pixel_width }
  render_cache.rs                 RenderCache (per-doc-line glyph instances + cluster_data)
  app.rs:140                      App.advance_cache (单帧 hit-test 缓存)

RESHAPE WORKER:
  app.rs:103,149                  reshape_worker, reshape_generation
  reshape_worker.rs:42-132        ReshapeWorker (spawn, submit, drain_completed, cancel_before, shutdown)
  reshape_worker.rs:135-192       process_with_shaper
  reshape_worker.rs:195-286       process_fallback
  app.rs:1312-1347                drain_reshape_results
  app.rs:1350-1398                submit_reshape_ahead

VIEWPORT:
  viewport.rs:116-127             Viewport { scroll_top, visible_rows, viewport_height, scroll_anchor }
  viewport.rs:151-187             first_visible_row, visible_display_range, visible_doc_line_range
  viewport.rs:189-218             scroll_by, scroll_to_row, clamp_scroll_top, clamp_scroll_top_no_wrap
  viewport.rs:228-253             sync_anchor_from_scroll, restore_scroll_from_anchor, refold_on_*

CURSOR / AUTOSCROLL:
  cursor_motion.rs:55-63          CursorContext
  cursor_motion.rs:260-310        move_cursor_visual (4a / 4b / 4c)
  cursor_motion.rs:136-183        move_up_past_visible
  cursor_motion.rs:186-255        move_down_past_visible
  app.rs:899-913                  App::move_cursor_visual
  app.rs:1400-1431                App::post_shape_update (autoscroll)
  document_view/mod.rs:556-577    DocumentView::ensure_cursor_visible
  document_view/mod.rs:580-601    page_up, page_down

INVALIDATION TRIGGERS:
  app.rs:354-422                  Zoom In/Out/Reset
  app.rs:850-894                  resize (width vs height-only)
  app.rs:1313-1347                drain_reshape_results
  app.rs:2068-2107                handle_command 编辑路径
  app.rs:2171-2213                IME commit 路径
  display_line_map.rs:88-107      set_viewport_size
```

### 测试位置

```
layout.rs:303-355                 wrap algo 单测（仅 long_unbreakable_token_after_space）
viewport.rs:271-845               Viewport 大量单测（display_row, scroll, anchor）
display_line_map.rs:197-300       DisplayLineMap 单测
snap_tree.rs:384-660              SnapTree 单测 + bench
reshape_worker.rs:288-379         worker 单测（**当前编译失败**）
render_pipeline_tests.rs          render_pipeline 单测
document_view/test_*.rs           各种集成测试
```

---

## 5. 优先修复建议

### P0（立刻）

1. **修 reshape_worker.rs 测试**：测试调 `ReshapeWorker::spawn()` 改成 `spawn("Menlo".into())` 或 `spawn(Settings::get_static().font_family.clone())`。
2. **编辑路径 cancel worker**：`handle_command` 在 `outcome.executed && outcome.dirty_lines.is_some()` 时 `reshape_generation += 1; worker.cancel_before(...)`。同样改 IME commit 路径。
3. **统一 `process_fallback` 与 `compute_visual_lines`**：worker 端 `process_fallback` 调用 `layout::compute_visual_lines` 自身——但 fallback 没有 shaped clusters。可以选择：(a) 把 fallback 路径删掉，shape 失败就 placeholder（让主线程渲染时 sync shape）；(b) 给 fallback 一个不依赖 shape 的简化版 wrap，但仍含词边界 + trim。

### P1（高优）

4. **Subset 模式更新 first_line/last_line**：`render_pipeline.rs:478` 的 `if i == 0 && !shape_subset_only` 条件去掉 `&& !shape_subset_only`（用 cached cluster_data 喂 LineCache）。
5. **render_cache 在 set_viewport_size 失效时一并失效**：`set_viewport_size` 触发 width/font 变就额外 invalidate_all 渲染缓存。
6. **解决 DPI 变化无 handler**：响应 `WindowEvent::ScaleFactorChanged`，重新 `Settings::apply_scale`、`init_display_map`、`reshape_generation += 1`、cancel_before。
7. **`16.0` 硬编码改为 `Settings::scrollbar_reserve()` 或类似命名**：跟 dpi_scale 联动。

### P2（架构）

8. **DisplayLineMap 提到 DocumentView 内部**：每个 dv 一个 display_map；worker 请求带 dv handle / generation 元组；切 tab 时 worker queue 清理只针对当前 dv。
9. **scroll_anchor 加 `visual_line_in_doc`**：解决 B-13。
10. **shape_fast 的字体选择跟 shape 对齐**：`shape_fast` 也走 `Family::Name(font_family)` 优先；fail 后再尝试 fallback families。
11. **autoscroll 提前到 render 之前**：跟 `displayrow.md` Phase 5 一致，移除 ensure_cursor_visible 双入口，让 autoscroll 在 shape_visible_lines 之前显式跑一轮（用上一帧的 cursor_visual_line_in_doc 估计）。

---

## 6. 附录 — 文档脚注

- `plans_wrap_algo_fix.md`：阶段 1-5 全部标 [x]，但 Task 8 的「极窄视口强制断」实际未在 layout.rs 体现（只看到 `ci > start` 守卫，没有 `ci == start` 时强制单簇成行的分支）。建议核对。
- `docs/displayrow_review.md`：Phase 5 / 6 仍有未完工项（ensure_cursor_visible 双入口、advance_cache DisplayRow、move_cursor_visual 4b/4c）。
- `docs/scroll_bugs_root_cause.md`：症状 1（滚轮闪缩）与症状 2（方向键不滚）都跟 `visible_range` 不补偿 offset、`advance_cache` 抖动相关；DisplayRow 重构理论上已修，但 `cursor_motion.rs::move_down_past_visible` 仍直接读 `advance_cache.last()` + `last_line.visual_lines`——subset 模式下 last_line 不更新，B-6 仍存在。
- `docs/viewport_architecture_analysis.md`：建议长期借鉴 Zed 的 DisplayMap + 锚点设计，本审计的 P2 建议跟它方向一致。
- `docs/plan-ui-split.md`：viewport 提到 `crates/ui` 的迁移会引入 `DisplayLineLookup` trait——本审计的 P2-#8（display_map 下沉到 dv）会与 ui split 计划冲突，需协调。

---

## 7. 工作量与回归风险评估

| 修复 | 工作量 | 回归面 |
|---|---|---|
| P0-1 修测试 | 5 min | 0 |
| P0-2 编辑 cancel | 30 min | 主路径 hash 检查兜底，回归低 |
| P0-3 fallback 算法统一 | 半天（要写一份不依赖 clusters 的 wrap） | 中（影响 shape 失败的兜底，正常环境不触发） |
| P1-4 subset first_line | 30 min | 移除 `!shape_subset_only` guard 后要确认 LineCache 写入用的 cluster_data 字段（cached.cluster_data 是 (start,end,adv)，跟 first_line.clusters 同型） |
| P1-5 RenderCache 跟 set_viewport_size 失效 | 10 min | 性能：width 变时多一次 invalidate，OK |
| P1-6 DPI handler | 1 小时 | 中（要 retest 多 DPI 场景） |
| P2-8 DisplayLineMap per-dv | 1-2 天 | 大（worker 协议改动） |

— END —
