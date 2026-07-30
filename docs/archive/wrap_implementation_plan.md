# 换行管线修复实施计划

> 基于 `docs/wrap_pipeline_audit.md`（578行审计）与当前代码现状。
> 工作树：`/Users/dan/.codex/worktrees/e010/edit+`
> 日期：2026-06-09

---

## 当前代码基线

| 文件 | 状态 | 与 git HEAD 差异 |
|------|------|-----------------|
| `layout.rs` | **干净**（刚才恢复） | 无 diff |
| `app.rs` | 有一行改动 | `+ self.needs_redraw = true` 在 tree_dirty 块后 |
| `reshape_worker.rs` | 有改动 | shape_fast 失败时先试 `shape()` 再 fallback |
| 编译 | `cargo check` **通过** | — |
| 测试 | `cargo check --tests` **失败** | 4 处 E0061：`spawn()` 零参调用 |

---

## P0 — 立刻修（阻塞编译 / 核心正确性）

### P0-1：修复 reshape_worker 测试编译

**问题**：`ReshapeWorker::spawn()` 签名改为 `spawn(font_family: String)`，测试仍以 `spawn()` 零参调用。

**位置**：`crates/app/src/reshape_worker.rs:318, 333, 349, 366`

**方案**：
```rust
// 改前
let w = ReshapeWorker::spawn();
// 改后
let w = ReshapeWorker::spawn("Menlo".into());
```

**验证**：`cargo check -p edit-plus-app --tests` 零错误

---

### P0-2：编辑路径与 IME 路径 cancel worker

**问题**：`handle_command` (行 2068) 和 IME commit (行 2186) 编辑后，没有 bump `reshape_generation` + `cancel_before`。异步 worker 队列中的旧请求（基于编辑前内容）不会被丢弃，结果会在 `drain_reshape_results` 中落地，覆盖编辑后新行的内容。

**位置**：`crates/app/src/app.rs` handle_command 函数末尾

**方案**：在 `if outcome.executed` 块内、`sync`/invalidate 之后，增加：
```rust
self.reshape_generation += 1;
if let Some(ref w) = self.reshape_worker {
    w.cancel_before(self.reshape_generation);
}
```

IME commit 路径同理（行 2186 附近）。

**注意**：确认 IME 路径也检查 `outcome.executed`，两个路径用同一逻辑。

**验证**：代码审查 + 编译通过。纯逻辑缺陷修复，无需运行时测试。

---

### P0-3：统一 process_fallback 与 compute_visual_lines 断行规则

**问题**：`reshape_worker.rs:process_fallback` 走完全不同的逐字符硬断：
```rust
// 当前代码（reshape_worker.rs:236-262）
for (ci, ch) in line_str.char_indices() {
    if line_px + ch_w > req.viewport_width && ci > byte_pos {
        breaks.push(...);  // 纯硬断，无任何规则
        byte_pos = ci;
        line_px = ch_w;
    }
}
```
- 不识别词边界（空格后断行偏好）
- 不 trim 空白
- 不在 ASCII alnum 串内部回溯
- 不处理 CJK 标点禁则
- 不处理 CJK 边界检测

一旦 worker 端 shape 失败 → fallback → 缓存里的 `visual_breaks` 与渲染端 `compute_visual_lines` 不一致。

**方案**：给 `process_fallback` 增加与 `compute_visual_lines` 一致的断行规则。

由于 fallback 没有 `shaped.clusters`，只能按 Unicode 字符边界遍历。关键规则：

```
1. 空格后断行偏好：追踪 last_space_byte
2. ASCII alnum 连续串保护：当硬断点在 alnum 串内部时，回溯到串开头（或最近空格）
3. 标点保护：断点后第一个非空字符若是标点，拒绝该断点
4. 续行 trim 前导空白
5. 宽度估算：ASCII char_w = font_size * 0.6，CJK char_w = font_size
```

**实现位置**：`crates/app/src/reshape_worker.rs` `process_fallback` 函数

**边界细节**：
- 用 `line_str.char_indices()` 遍历，`ci` 是字节偏移，`ch` 是 char
- `CJK` 判断：`crate::layout::is_cjk_char(ch)`
- 标点判断：`ch.is_ascii_punctuation()`
- 连续串回溯：从 `ci` 往回扫描直到遇到非 alnum 或空格

**验证**：新建测试，覆盖 ASCII 长数字串、空格+标点、CJK 混合行

---

## P1 — 高优（断行质量 / 缓存正确性）

### P1-4：降低 ASCII alnum 回溯门槛

**问题**：`compute_visual_lines` 中 ASCII alnum 回溯条件：
```rust
if run_start_x >= hard_x * 0.3 {  // ← 短前缀不触发
    break_at = run_start;
}
```
短前缀场景（如 `"ID: 1234567890..."`）时 `run_start_x` 可能只有 `~30px`，`hard_x` 有 `200px`，`30 < 60` → 不触发 → 数字仍然断开。

**方案 A（保守）**：改 `0.3` 为 `0.15` 或 `char_width * 2.0`  
**方案 B（激进）**：去掉下限，只要 `run_start > start` 就回溯  
**推荐**：方案 B。回溯点一定 ≥ `start` 且是合法的 alnum 串起点，短前缀也比中间断开数字好。

**位置**：`crates/app/src/layout.rs` 行 ~260

**验证**：新增测试 `ascii_number_with_short_prefix`：前缀 3 个字符 + 空格 + 长数字串，断言数字串不被从中断开。

---

### P1-5：Whitespace candidate 增加标点保护

**问题**：当首选断点是空格边界 (`cand_ws`)，没有检查断点后第一个 cluster 是否为标点。导致：

```
But then, she said...
→ "But then,"   ← 正确
→ ", she said"   ← 逗号单独在行首
```

CJK candidate 已有此保护，需对 `cand_ws` 也加。

**方案**：在 `cand_ws` 的 `if let Some(i)` 块内，先检查 `clusters[i]` 是否为标点：
```rust
if let Some(i) = cand_ws {
    let next_is_punct = line_bytes.get(clusters[i].byte_range.clone())
        .map(is_punctuation).unwrap_or(false);
    if !next_is_punct {
        let ws_x = trimmed_width(start, i);
        let accept = if in_cjk { ws_x >= best_x } else { ws_x >= hard_x * 0.5 };
        if accept { break_at = i; best_x = ws_x; }
    }
}
```

**位置**：`crates/app/src/layout.rs` 行 ~276

**验证**：新增测试 `ascii_punct_not_alone_at_line_start`：构造 `"hello, world, foo"` 在窄视口断行，验证逗号不出现在行首。

---

### P1-6：Subset 模式修复 first_line/last_line 更新

**问题**：`render_pipeline.rs:478` 的 `if i == 0 && !shape_subset_only` 条件限制了 subset shaping 时不更新 `first_line`。导致使用 `first_line.visual_lines` 的光标导航（`move_cursor_visual`、`cursor_motion.rs:move_down_past_visible`）在长行 subset 模式下使用过时数据。

**方案**：去掉 `&& !shape_subset_only` 条件，用 `cached.cluster_data` 类型 (`(usize, usize, f32)`) 构造 `first_line.clusters`。

**位置**：`crates/app/src/render_pipeline.rs` 行 478

**验证**：在渲染缓存存在的长行中做光标上下移动，确认光标列位置正确。

---

### P1-7：RenderCache 在 set_viewport_size 触发失效时一并失效

**问题**：`set_viewport_size` 会在 width/font_size 变化时把 `entries.visual_breaks.clear()` + `visual_line_count=1`，但没有 invalidate `RenderCache`。下一帧渲染时，可能缓存命中返回旧的 `visual_lines`（带着旧的换行结果），虽然 `content_hash` 通常会因为 `font_size.to_bits()` 变化而 miss，但 width-only 变化时 hash 中只含 font_size 不含 width。

等等——检查 `content_hash_fast` 在 `render_pipeline.rs:436`：
```rust
let content_hash_fast = {
    let off = dv.line_byte_offset(doc_line_idx).unwrap_or(0);
    let len = dv.line_byte_length(doc_line_idx).unwrap_or(0);
    (off as u64)
        .wrapping_mul(31).wrapping_add(len as u64)
        .wrapping_mul(31).wrapping_add(viewport_width.to_bits() as u64)  // ← 含 width
        .wrapping_mul(31).wrapping_add(Settings::get().font_size.to_bits() as u64)
};
```

width 在 hash 里，所以它**会** miss → 安全。但安全防御最好还是加上 explicit invalidate。

**方案**：在 `display_line_map.rs:set_viewport_size` 末尾加注释说明 RenderCache 不在此处失效的原因（因为 content_hash 含 viewport_width），但安全起见，在 `app.rs` 的 resize 路径中已调 `render_cache.invalidate_all()`（行 865），不需要额外改动。

**建议**：此条改为**备忘项**，不修改代码。

---

## P2 — 架构改进（延后，不在本次实施）

| 条目 | 说明 | 工作量 |
|------|------|--------|
| P2-8 DisplayLineMap per-dv | 每个 DocumentView 独立一份 | 1-2 天 |
| P2-9 DPI handler | ScaleFactorChanged event | 1 小时 |
| P2-10 16.0 硬编码 | 提取为 Settings::scrollbar_reserve | 30 分钟 |
| P2-11 shape_fast 字体对齐 | 优先 Family::Name | 1 小时 |

---

## 实施顺序

```
Phase 1（编译 + 核心正确性）：
  ├── P0-1 修测试编译         ← 5 分钟，无风险
  ├── P0-2 编辑/IME cancel    ← 30 分钟，低风险
  └── P0-3 process_fallback   ← 半天，中风险

Phase 2（断行质量）：
  ├── P1-4 ASCII alnum 阈值   ← 15 分钟，低风险
  ├── P1-5 标点保护          ← 15 分钟，低风险
  └── 测试补充               ← 1 小时

Phase 3（缓存 + 导航）：
  ├── P1-6 subset first_line  ← 30 分钟，中风险
  └── P1-7 确认安全（备忘）   ← 不修改

Phase 4（验证）：
  └── 编译 + 全量测试        ← 30 分钟
```

---

## 风险评估

| 改动 | 回归面 | 缓解措施 |
|------|--------|----------|
| P0-2 cancel worker | 理论上安全（generation 机制兜底），但可能漏 cancel 导致闪烁一帧 | generation 递增后旧结果在 drain 时被过滤 |
| P0-3 fallback 重写 | 中等——影响 shaper 初始化失败的罕见场景 | 保留原 `if ascii_w <= 0.0` 快速路径；新逻辑从 `compute_visual_lines` 提取核心规则 |
| P1-4 降低阈值 | 极低——只影响 ASCII alnum 回溯条件 | 仅放宽条件，不改变逻辑结构 |
| P1-5 标点保护 | 低——只在 whitespace candidate 路径增加 guard | 与 CJK candidate 对标点保护一致 |
| P1-6 subset first_line | 中等——影响光标上下导航 | 确认数据结构兼容后再改 |

---

## 关键文件清单

| 文件 | 本次修改 | 说明 |
|------|----------|------|
| `crates/app/src/reshape_worker.rs` | ✅ | P0-1 测试 + P0-3 fallback |
| `crates/app/src/app.rs` | ✅ | P0-2 cancel worker |
| `crates/app/src/layout.rs` | ✅ | P1-4 阈值 + P1-5 标点 |
| `crates/app/src/render_pipeline.rs` | ✅ | P1-6 subset first_line |
| `crates/app/src/reshape_worker.rs` (测试) | ✅ | 新增 fallback 测试 |
| `crates/app/src/render_pipeline_tests.rs` 或 `layout.rs` (测试) | ✅ | 新增断行质量测试 |
