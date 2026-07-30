# 大文件 Tab 切换位置漂移 — 修复方案

## 一、问题现象

打开大文件（50k+ 行，CJK 较多）后，**反复在 tab 之间来回切换**，每次切回同一个 tab，文本视觉顶端的位置都会偏移几行到十几行。小文件无此现象。

## 二、根因分析

### 2.1 数据结构

- `display_map`：每个 doc_line 一个 `DisplayLineEntry`，含 `visual_line_count`（折行后占多少行）。
  - **远处行**最初是 placeholder：用 `est_chars_per_line` 估算的 vl（CJK 估值偏小）。
  - reshape worker 在后台把 placeholder 替换成真实 vl，并 `rebuild_tree()` 重算 SnapTree 的累加和。
- `scroll_top`：一个 `f64`，表达"视口顶端落在第几个 display_row"（折行后的行号空间）。
- `scroll_anchor = (doc_line, pixel_offset)`：滚动位置的语义化表达，pixel_offset 是当前 doc_line 内部的行内偏移。

两者通过 `display_map` 的累加和互转：
```
sync   :  scroll_top → anchor      (用当前 display_map 的累加和)
restore:  anchor     → scroll_top  (用当前 display_map 的累加和)
```

### 2.2 漂移发生的时序

**fast path 的失效**（`crates/app/src/app.rs:380-404`）：

```rust
if dv.display_map.line_count() == dv.line_count() && dv.line_count() > 1 {
    if let Some(first) = dv.display_map.get_entry(0) {
        // …只检查 doc_line=0 的 content_hash 是否匹配
        if !is_placeholder && first.content_hash == expected_hash {
            dv.viewport.restore_scroll_from_anchor(...);   // ← 走 fast path
            return;
        }
    }
}
```

fast path 的判定**只检查 doc_line=0**。但远处行（placeholder）可能在 tab 切走期间被部分 reshape，也可能没有；`map_doc_to_display(anchor.doc_line)` 是从 0 累加到 anchor.doc_line 的全局求和，**前面任何一行的 vl 变化都会改变这个累加值**。

完整时序（导致每次切回漂移）：

| 步 | 事件 | display_map 状态 | scroll_top |
|---|---|---|---|
| 1 | 用户在 tab A 滚动到 doc_line=1000，渲染稳定 | 视口附近真实 vl，远处 placeholder | X |
| 2 | reshape worker 异步算完远处某些行 | 部分远处 placeholder → 真实 vl | X |
| 3 | drain_reshape_results 触发 rebuild_tree | mapping 变了 | clamp 后仍 ≈ X |
| 4 | drain 内部的 sync_anchor_from_scroll | mapping_3 | anchor = (1000, off_3) |
| 5 | 切走 → 切到 tab B → 切回 tab A | (display_map A 不变) | — |
| 6 | handle_workspace_effect → invalidate_reshape → init_display_map fast path | 走 fast path（doc_line=0 hash 一致） | restore: scroll_top = map_3(1000) + off_3 ≈ X |
| 7 | submit_reshape_ahead 重新提交 visible 范围 | 进入 pending | X |
| 8 | 几帧后 drain 收到结果，rebuild_tree | mapping_8 (远处 placeholder 又变了) | clamp 后仍 ≈ X |
| 9 | drain 内部 sync_anchor_from_scroll | anchor = (998, off_9) ← **doc_line 被改写了** | X |

**关键漂移点**：
- 步 8 `rebuild_tree` 后，`scroll_top=X` 在新 mapping 下对应**别的 doc_line**（视觉位置变了）
- 步 9 `sync_anchor_from_scroll` 用 `map_display_to_doc(scroll_top.floor())` 反查 doc_line，得到的是 mapping_8 下的新 doc_line（不是用户原来锚定的 1000）
- 下一轮切走/切回，anchor 已经被"污染"，每次切换累计偏移

### 2.3 当前代码的内部矛盾

| 位置 | 调用 | 假设的 source of truth |
|---|---|---|
| `app.rs:400` (fast path) | `restore_scroll_from_anchor` | anchor 是真实的 |
| `app.rs:1044` (drain rebuild) | `clamp_scroll_top` 不调 restore | scroll_top 是真实的 |
| `app.rs:1041` 注释 | "DO NOT restore_scroll_from_anchor" | scroll_top 是真实的 |
| `app.rs:1241` (滚动后) | `sync_anchor_from_scroll` | scroll_top 是真实的 |
| `app.rs:907` (resize 时) | `sync_anchor_from_scroll` | scroll_top 是真实的 |
| `app.rs:924` (resize 后) | `restore_scroll_from_anchor` | anchor 是真实的 |

**两端拉锯**：滚动事件之后 anchor 跟随 scroll_top；fast path 反过来 anchor 决定 scroll_top；drain 又只动 scroll_top 不动 anchor（实际上 step 9 仍然会被后续 scroll/resize 触发的 sync 改写）。

placeholder 期内 mapping 频繁变化，每变一次都让 anchor / scroll_top 之一被"污染"，反复切 tab 累积漂移。

## 三、设计目标

1. **切走 → 切回**视觉位置稳定（用户看到的同一行）。
2. **大文件首次切回不卡帧**：reshape 是后台、异步的，不能等全部完成才允许切。
3. 不破坏现有的滚动条 thumb、page up/down、cursor 跟随等行为。
4. 修复路径要可分阶段验证，每个阶段独立可上线。

## 四、方案

### 总原则

```
anchor 是唯一持久化的滚动状态。
scroll_top 是派生量，每帧从 anchor 计算得到。
display_map 变化时不允许写 anchor，只允许重新派生 scroll_top。
```

只要 anchor 不被 reshape 过程污染，无论远处 vl 怎么变，"用户看到的顶端 doc_line"始终是 anchor.doc_line —— 视觉锚定。

### 阶段切分

按"接口先行、逐步替换 source of truth、最后再考虑渲染重构"切分。每阶段完成后立刻可验证大文件切 tab 漂移是否减小。

---

### 阶段 1：诊断与基线（0.5 天）

**目标**：用日志/统计验证根因，并提供回归基线。

1. 在 `sync_anchor_from_scroll` / `restore_scroll_from_anchor` / `rebuild_tree` 各加一条 trace（feature flag 控制，默认关）：
   - 输出当前 `(scroll_top, anchor.doc_line, anchor.pixel_offset, map_doc_to_display(anchor.doc_line))` 四元组。
2. 准备一个 50k 行 CJK 测试文本，写一个手动测试脚本：打开 → 滚到 25000 行 → 切到 tab 2 → 切回 → 切走 → 切回，重复 10 次，记录每次切回后的 anchor.doc_line / scroll_top。
3. 把基线（漂移幅度）记到 `docs/manual_test_protocol.md`。

**完成判定**：log 能直接定位是哪一次 sync_anchor 把 doc_line 改写了。

---

### 阶段 2：建立 anchor 单向写入规则（1 天，独立）

**目标**：禁止 reshape / display_map 变化路径写 anchor，只允许"用户视口意图"写 anchor。

具体改动：

1. **`drain_reshape_results`（`app.rs:1011-1049`）**：
   - 移除 rebuild_tree 之前的 `sync_anchor_from_scroll`（a1a476b 引入的那一行）。
   - rebuild_tree **之后**改为 `restore_scroll_from_anchor`，让 scroll_top 跟随 anchor 在新 mapping 下重新派生。
   - 紧跟 `clamp_scroll_top`（仅边界修正，不再 sync）。
2. **`handle_resize`（`app.rs:880-929`）**：
   - 移除 `:907` 的 `sync_anchor_from_scroll`（resize 不应改写 anchor）。
   - 保留 `:924` 的 `restore_scroll_from_anchor`。
3. **新增辅助方法**：在 `Viewport` 上提供 `update_anchor_from_user_intent(map, line_height)`，只有用户主动滚动 / page / cursor 跟随的路径才允许调用，内部就是当前的 `sync_anchor_from_scroll`。重命名旧名字以防误用，或加 `#[doc(hidden)]` 警示。
4. **scroll handler（`app.rs:1238-1242`）保留**：用户滚动后写 anchor 是合法路径。
5. **clamp 不再触发 sync**：`Viewport::clamp_scroll_top`（`viewport.rs:212-220`）当前末尾会 `self.sync_anchor_from_scroll`，这是 anchor 被污染的另一条路径。改为：
   - 只 clamp scroll_top 数值
   - **不调用 sync**
   - 由调用方决定是否需要在 clamp 后单独同步 anchor（仅用户事件）

**测试**：阶段 1 的 trace 应当显示：reshape / resize 路径下 anchor.doc_line 不再改变。

---

### 阶段 3：fast path 判定收紧（0.5 天，独立）

**目标**：`init_display_map` fast path 当前只查 doc_line=0 的 hash，对 placeholder 是否完整不敏感。

1. fast path 不仅检查 doc_line=0，还要确认**整个 display_map 没有 placeholder 残留**（或至少 anchor.doc_line 附近 ±visible_rows 内没有 placeholder）。
   - 增加 `display_map.has_placeholder_in_range(start, end) -> bool`。
2. 如果残留 placeholder，**不**走 fast path，但也**不**重建（重建会丢掉已 reshape 的真实 vl，浪费）。
   - 改为：保留现有 entries，仅重新触发 reshape worker 把 placeholder 补齐，scroll_top 暂保持上次保存的值（从 snapshot 恢复）。
   - 等阶段 4 的 drain 路径把 scroll_top 校正为 anchor 推导值。

**测试**：切大文件 tab，反复切换，scroll_top 数值在 reshape 期间会有 ±数行的派生变化（这是正确的——为了保 doc_line），但**视觉顶端的 doc_line 始终是 anchor.doc_line**。

---

### 阶段 4：snapshot 与 lazy_load_tab 路径补齐（0.5 天，独立）

**目标**：跨进程持久化的 anchor 路径与运行时一致。

1. `workspace.rs:175-204` `lazy_load_tab` 现在用 `scroll_anchor_line + scroll_anchor_offset/lh` 直接给 scroll_top 赋值，等价于"如果每行 vl=1 时的反算"。在大文件上首次显示时这个值是错的，要等首次 reshape 后由阶段 2 的 drain 路径校正。
   - 验证此路径不会再写 anchor（lazy_load 把 stub 的 anchor 复制过来即可，无需 sync）。
2. `workspace.rs:515` 同样的 snapshot 恢复路径补齐。
3. `app.rs:580` 直接 scroll_top 赋值的路径（cursor 恢复）改为：先设 anchor，再 restore。

**测试**：冷启动恢复 50k 行文件 + 滚动到 25000 行的 snapshot，启动后视口正确落在 25000 行。

---

### 阶段 5（可选，长期）：anchor-based 渲染（3-5 天）

**目标**：彻底消除 mapping 累加和的不稳定性影响，参考 Zed 的做法。

当前渲染依赖 `scroll_top.floor()` 作为 visible 起点。如果改为：

```
visible_top_doc_line = anchor.doc_line
visible_top_pixel    = anchor.pixel_offset
按 visible_rows 从 anchor.doc_line 向下绘制（无需查 map_doc_to_display）
```

则视觉顶端永远精确锁定 anchor，跟远处 placeholder 完全解耦。代价：
- 滚动条 thumb 位置仍需估算（用 anchor.doc_line / total_doc_lines 近似，placeholder 期间 thumb 位置可能略有跳动，但这是次要 UX）。
- page up/down 不再用 scroll_top ± visible_rows，改为用 anchor 步进（`anchor.doc_line` 减去若干行）。
- 涉及多个 UI 模块，需要先在 `Viewport` 抽象出 `visible_rows_from_anchor(map, line_height) -> impl Iterator<DocLine>`，然后逐个替换调用点。

阶段 1-4 完成后即可验证视觉漂移消失；阶段 5 是为更彻底的稳定性、为未来虚拟滚动/极大文件做的架构演进，不是修复必需。

## 五、接口与协议

### 阶段 2 新增/重命名

```rust
// crates/ui/src/viewport.rs
impl Viewport {
    /// 仅供"用户主动改变视口"路径调用：滚动事件、page up/down、cursor 跟随。
    /// 不要在 reshape / resize / drain 路径调用。
    pub fn update_anchor_from_user_intent(&mut self, map: &impl LineMap, line_height: f32);

    /// 边界修正，不再 sync anchor。
    pub fn clamp_scroll_top(&mut self, map: &impl LineMap, line_height: f32);

    /// 用 anchor + 当前 mapping 派生 scroll_top。display_map 变化后调用。
    pub fn restore_scroll_from_anchor(&mut self, map: &impl LineMap, line_height: f32);
}
```

旧的 `sync_anchor_from_scroll` 标记为 `#[deprecated]` 并在所有非用户事件路径删除调用。

### 阶段 3 新增

```rust
// crates/app/src/display_line_map.rs (or snap_tree)
impl DisplayLineMap {
    pub fn has_placeholder_in_range(&self, doc_start: usize, doc_end: usize) -> bool;
}
```

## 六、风险与回滚

- 阶段 2 移除 sync 调用后，如果某些非用户事件路径还期望"clamp 自动同步 anchor"，会出现 cursor 跳错位置等回归。**缓解**：阶段 2 完成后用现有测试套（`document_view/test_cursor_visual_tests.rs`、`commands.rs` 中的 page up/down 测试）作为回归门禁。
- 阶段 5 重构涉及面广，单独做一次主分支合并；阶段 1-4 可以合并到主分支后单独评估是否进入阶段 5。
- 每个阶段完成后阶段 1 的 trace 工具不要立刻删，作为后续观测手段保留至少 1-2 个版本。

## 七、验收清单

- [ ] 阶段 1：trace 输出能定位 anchor 被污染的具体调用栈
- [ ] 阶段 2：50k 行 CJK 文件、反复切 tab 10 次后，`anchor.doc_line` 与第 1 次切走前完全相同
- [ ] 阶段 3：fast path 命中率（log 统计）≥ 90%；未命中时 reshape 完成后视觉位置正确
- [ ] 阶段 4：冷启动恢复 snapshot 后视口精确落在保存位置
- [ ] 阶段 5（可选）：scroll_top 完全成为派生量，没有任何 `dv.viewport.scroll_top = ...` 直接赋值
- [ ] 现有测试套全部通过：`cargo test -p edit_plus_app`
