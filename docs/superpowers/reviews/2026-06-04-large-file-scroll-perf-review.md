# 大文件滚动性能 — 实施审查报告

> 审查日期：2026-06-04
> 范围：`docs/superpowers/specs/2026-06-03-large-file-scroll-perf-design.md` + `docs/superpowers/plans/2026-06-03-large-file-scroll-perf.md` 对应代码（commits `744a252..a81d4e2`）
> 审查方法：spec/plan 与现仓代码逐条对照 + `cargo check` + `cargo test -p edit-plus-app`

## 总评

整体判断：**Phase 1-3 部分完成且偏离设计；Phase 4-5 基本未做**。RenderCache 命中路径已搭出来，但 spec 中"对齐 Zed DisplayMap"的核心机制（异步 worker、ScrollAnchor 替换 scroll_top、atlas 驱逐反向索引）多数缺位或仅留装饰性骨架。已存在一处 golden image 回归。

## 完成度对照（按 spec §9 五阶段）

| 阶段 | 设计稿要求 | 实际状态 |
|---|---|---|
| 1. SnapTree | 持久化 B-tree、splice O(k log n) | 数据结构骨架在；**`splice` 全量 `entries.clone()` + `from_entries` 重建**（`snap_tree.rs:155-182`），不是增量 |
| 2. DisplayLineMap + ReshapeWorker | sync 双路、worker generation 三层校验、cancel_before、背压、poll_worker | DisplayLineMap 只有 252 行（设计 650），仅 `sync(range, replacements)`；**ReshapeWorker 在 `app.rs` 完全未集成**，只在自身单测里 spawn；`cancel_before` 是空函数；无 generation 校验、无背压、无 poll_worker、无 set_viewport_size 回写 |
| 3. RenderCache + 顶点重构 | 行内相对坐标 + 反向索引 + atlas_generation 联动 + 主题独立 | RenderCache 主体在，主题独立成立；**反向索引、`insert_with_eviction`、atlas LRU 驱逐 invalidation 全部缺失**；`atlas_generation` 写死 0 |
| 4. ScrollAnchor | scroll_top → ScrollAnchor 替换、编辑/resize 不漂 | **完全未替换**：Viewport 仍以 `scroll_top: f64` 为真值；`scroll_anchor` 字段是死字段；`mouse.rs / scrollbar.rs / commands.rs` 0 处引用；`adjust_after_edit` 未实现 |
| 5. 清理 + 收尾 | 删 WrapIndex、删 shape_cache/wrap_cache、超长行开关、resize 16ms 节流 | shape_cache/wrap_cache 已删（✅）；**WrapIndex 仍是 900 行且 `render_pipeline` 多处依赖**；`Settings::max_line_bytes_for_shaping` 字段在但**从未被引用**；`pending_resize / last_resize_handled` 字段已加但未接入事件循环 |

## 关键缺陷（按风险分级）

### Critical — 必须修

1. **`render_smoke` 测试 FAILED**（`crates/app/tests/render_smoke.rs:467`）

   ```
   SSIM = 0.8451 < 0.95: rendered image differs too much from golden
   ```

   说明 RenderCache 路径或高亮颜色映射改动了实际渲染输出，是**未受控的回归**。
   需要找到根因（怀疑 cache hit 与 cache miss 两条分支的颜色处理不一致：cache miss 走 `highlight_color_for_offset`，cache hit 走 `highlight_kind_to_color`，二者对"无 span"和"边界 cluster"的处理差异）。

2. **ReshapeWorker 是悬空模块** — 设计稿核心组件之一，但 `app.rs` 从不 `spawn`。这意味着大编辑（粘贴 5000 行）不存在异步路径，全部同步走 `display_map.sync` + 主线程 shape，spec §5.4 的目标未达成。

   ```
   $ grep -rn "ReshapeWorker::spawn" crates/app/src/  # 只有 reshape_worker.rs 的单测
   ```

3. **`DisplayLineMap.sync` 在编辑路径只塞 placeholder**（`app.rs:1809-1814`）—— 真正的 wrap 数据靠下次 `render_pipeline` 命中时 `update_entry_in_place` 回填。这意味着编辑后到下次该行进入 viewport 之前，`display_map` 的 `total_rows` 会失真（每行算 1 个 vl），spec §6.5 不变量 I2 在中间窗口被破坏。

### Important — 应当修

4. **`DisplayLineMap.sync` 的 `affected_lines = 0..self.line_count`**（`display_line_map.rs:120`），注释直说"简化：mark all affected"。`app.rs` 没用这个字段（用的是 `outcome.dirty_lines`），但 `DisplayPatch` 因此变成误导性 API。

5. **ScrollAnchor 是装饰性字段**：spec §1 目标 2「编辑后视口锚定行不漂」未达成。`viewport.rs:223-246` 的 `sync_anchor_from_scroll/restore_scroll_from_anchor/refold_on_edit` 都没在编辑/resize 路径上调用过。`refold_on_resize` 还**写死 `line_height=14`**（`viewport.rs:233`）。

6. **rapid_cooldown 始终为 0**（`app.rs:1284-1292`）：

   ```rust
   if is_rapid_scroll { self.rapid_cooldown = 0; }   // 注释写"0 frames"，赋值也是 0
   ```

   字段冷却机制是空操作，`in_rapid` 仅在那一帧 `scroll_delta > visible_rows` 时为真。是否符合预期需要确认（注释说"instant recovery with O(1) display_map update"，意思可能是有意去掉冷却，那字段本身可以删）。

7. **三处 `content_hash` 算法不一致**：
   - `render_pipeline.rs:316` `(off*31 + len)`
   - `reshape_worker.rs:178/227` 前 32 字节累加
   - `snap_tree.rs:32` placeholder 写 0

   现在能跑只是因为 `CachedLine.content_hash` 字段没人比对，编辑通过 `invalidate_range` 显式失效。一旦后续启用 hash 校验路径，三者无法互通。

8. **`DisplayLineMap.snapshot()` 多包了一层 Arc**（`display_line_map.rs:57`）：`Arc::new(self.tree.clone())`，而 `SnapTree.clone` 内部已经是 `Arc::clone(root)` 的浅拷贝。这层 Arc 没有共享语义，每次 snapshot 都新建。设计 §6.5 不变量 I3 仍成立但代价加倍。

9. **`SnapTree::splice` O(n) 重建整棵树**（`snap_tree.rs:155-182`）。性能 bench 里 16 次 splice 没出问题是因为 LEAF_MAX=32 在 18000 行下树极浅，但任何编辑都会全量 `iter_lines + collect + from_entries`，违反设计 §4.1 的 O(k log n) 承诺。

### Minor — 影响代码卫生

10. **死字段 / 死代码**：
    - `Settings::max_line_bytes_for_shaping` 字段定义但从未读取（`render_pipeline` 写死 `MAX_WRAP_BYTES = 50_000`）
    - `App::pending_resize`、`App::last_resize_handled` 字段在但未接入 resize 事件
    - `snap_tree::collect_all_entries` 死代码（编译 warning）
    - `render_cache::highlight_color_for_offset` import 未用（warning）
    - `RenderCache.estimated_bytes / estimated_memory` 仅插入时累加，从未读取
    - `CachedLine.content_hash / atlas_generation / visual_line_count` 三个字段写入后从不比对

11. **`ScrollAnchor::pixel_offset` 实际单位混乱**：`sync_anchor_from_scroll` 把 `sub_row` 当 pixel_offset 直接存（`viewport.rs:227`），但变量是 `f64` 的小数行数；`refold_on_resize` 又当像素除以 14.0。语义不一致。

12. **GlyphInstance 实际大小约 56 B**（9 × f32 + u32 + u8 + padding），不是设计稿描述的 24 B；`MAX_CACHED_LINES=1000` × 平均 80 字符/行 → 约 4.5 MB，超出 §1 目标 5 的 ~3 MB 预算（但仍可接受）。

## 风险登记

| 风险 | 严重度 | 说明 |
|---|---|---|
| render_smoke 失败 | 高 | golden image 不匹配，可能用户实际看到字符颜色错乱（尤其无高亮 span 的纯文本） |
| 大编辑卡顿 | 高 | ReshapeWorker 未启用，spec §1 目标 4「粘贴 5000 行不阻塞」未达成 |
| 编辑后视口漂移 | 中 | ScrollAnchor 未接入；spec §1 目标 2 未达成 |
| atlas LRU 驱逐导致黑块/错字 | 中 | `atlas_generation` 永远是 0；驱逐发生时 RenderCache 字形 UV 已失效但仍被复用 |
| WrapIndex / DisplayLineMap 双轨数据漂移 | 中 | 两个数据结构在编辑路径都被独立 mutate，长时间交互下行号映射可能出现一行差 |
| 测试覆盖不足 | 中 | spec §8 列出的 `scrolling_does_not_call_shaper / atlas_eviction_invalidates_only_affected_lines` 等关键集成测试均缺失 |
| 验收记录未填写 | 低 | plan 末尾 §验收记录 仍是 `<填 ms>`，没有 4MB / 30ms / 0 invalidate 的实测数据 |

## 编译/测试结果

- `cargo check`：通过，3 条 warning（unused_import / unused_variable / dead_code，见上 §10）
- `cargo test -p edit-plus-app`：单元测试通过；**集成测试 `render_smoke::render_hello_to_png` 失败**（SSIM 0.8451 < 0.95）

## 建议下一步

按 CLAUDE.md 第 4 条「超过 3 文件先停下拆任务」，建议拆成 3 个独立小任务：

1. **修 render_smoke 回归** — 找到 cache hit/miss 颜色分歧 → 一文件改动，可优先做。
2. **接入或删除 ReshapeWorker** — 选其一：要么真正在 `app.rs` 接 `spawn` + `poll_worker`（按 §5.1/§5.4 时序），要么先把 `reshape_worker.rs` 标 `#[allow(dead_code)]` 并在 plan 标注延后。
3. **死字段清理 + WrapIndex 退场决策** — Phase 5 单独拉一支，要么按计划删 WrapIndex，要么明确 plan 范围降级为「双轨保留」。
