# 视觉行光标修复 + 折行管线统一方案

## 问题根因

系统中存在**两套独立的折行/布局系统**，字符宽度单位不同、断行算法不同：

| 维度 | TextBuffer (core) | 渲染管线 (app) |
|------|-------------------|---------------|
| 字符宽度单位 | 列（1-2，Unicode 表） | 像素（Shaper advance） |
| 断行阈值 | `word_wrap_column`（列数） | `viewport_width`（像素） |
| 断行算法 | 词边界感知（wrap opportunity） | 硬断行 |
| 启用状态 | `word_wrap_enabled=false`，从未启用 | 无条件折行 |

**实际表现**：
- 受影响：Home/End、Shift+Home/Shift+End、←/→ —— 命令路径走 TextBuffer 的视觉模型，但 word_wrap 从未启用，全部退化到文档行
- 未受影响：↑/↓（`cursor_motion.rs` 走 `advance_cache`）、鼠标点击（`mouse.rs:41`）、选区渲染（`render_geom.rs:42`）

`commands.rs:64`、`commands.rs:204` 的 `is_word_wrap_enabled()` 分支是死代码——永远走 else 分支。

---

## 方案分层

把工作切成两条独立路径，**优先做 Path A**：

- **Path A（MVP）**：修复用户能感知到的 bug，约 80 行代码改动
- **Path B（结构重构）**：等需要引入 inlay hint / fold / virtual block 等更复杂特性时再做

---

# Path A：视觉行光标修复（MVP）

**目标**：让 Home/End/Shift+Home/Shift+End 与渲染折行一致；引入词边界感知；清理 TextBuffer 的 word_wrap 死代码。

**总改动估算**：≤ 150 行，不引入新模块、不改类型签名。

## A1：渲染管线引入词边界感知

**文件**：`crates/app/src/render_pipeline.rs`

**改动点**：仅修改 `compute_visual_lines()` 内部逻辑，签名和返回类型不变。

参考 Zed `LineWrapper::wrap_line()` 的算法：

```
遍历 clusters，累积 x 位置：
  1. 在词边界（空格后第一个非空格/CJK 字符）记录候选断点
  2. 当 x + advance > wrap_width 时回退到最近候选断点
  3. 无候选断点（如超长无空格行）则退化为硬断行（保持现有行为）
```

**关键约束**：
- `wrap_width` 必须与 `wrap_cache` 的 key（`viewport_width.to_bits()`、`char_width.to_bits()`）逐位一致，否则缓存命中失败导致每帧重 wrap
- 词边界判定：ASCII 空格后的第一个非空格字符是断点；CJK 字符之间是断点

**测试**：在 `render_pipeline.rs` 末尾加 `#[cfg(test)] mod tests`，覆盖：
- ASCII 词边界折行
- CJK 折行
- 中英混排
- 超长无空格行（退化硬断行）
- 缓存 key 命中（同一字符串 + 同一 width 不重算）

**验收**：单测通过；运行 `cargo run` 实际折行从硬断改为词边界断（人工目视确认）。

## A2：Home/End/Shift+Home/Shift+End 走 advance_cache

**文件**：`crates/app/src/commands.rs`

`advance_cache` 已在 `app.rs` 持有，每帧渲染后是最新的。每条 entry 含 `doc_line + vl_byte_start + clusters[(byte_end, x)]`，足够定位光标所在视觉行。

**新增帮助函数**（建议放 `commands.rs` 或 `cursor_motion.rs`）：

```rust
/// 返回光标所在视觉行的字节范围 (line_abs_start, line_abs_end)。
/// 若 advance_cache 中找不到（光标不在可见区），回退到当前文档行的 [start, end]。
fn cursor_visual_line_bounds(
    dv: &DocumentView,
    advance_cache: &[AdvanceCacheEntry],
) -> (usize, usize);
```

实现要点：
- 用 `dv.cursor_offset` 找到对应 entry：匹配 `entry.doc_line == cursor_line` 且 `vl_byte_start ≤ local_offset ≤ last_cluster.byte_end`
- 不在可见区时回退到 `line_byte_offset(line)..+line_byte_length(line)`，与现有 `cursor_move_to_line_end` 行为一致

**改造 4 个分支**（`commands.rs:46-76`、`commands.rs:200+` 区域）：

| 命令 | 现在 | 改为 |
|------|------|------|
| MoveToLineStart（Home） | `cursor_move_to_line_start()` | 用视觉行 start，保留 indent 双击逻辑 |
| MoveToLineEnd（End） | `is_word_wrap_enabled()` 死分支 → `cursor_move_to_line_end()` | 用视觉行 end |
| ExtendToLineStart（Shift+Home） | `line_index.offsets[line]` | 用视觉行 start |
| ExtendToLineEnd（Shift+End） | `is_word_wrap_enabled()` 死分支 → 文档行 end | 用视觉行 end |

注意：indent-aware Home 的「连按两次回行首」语义保留，只是把「行首」从文档行改为视觉行。

**API 改造**：`execute_edit_command(cmd, dv)` 现在签名只有 `dv: &mut DocumentView`，需要追加一个 `advance_cache: &[AdvanceCacheEntry]` 参数（或把 advance_cache 移到 dv 中，由 app 帧末写入 —— 这两选一，前者改动小）。

**测试用例**（必须覆盖）：
- 短文档行（不折行）：Home/End 等价于文档行首/尾
- 长文档行（折行 N 段）：光标在第 i 段，Home/End 落在第 i 段的 start/end
- 光标在第 1 段时 Home：到第 1 段 start（也即文档行首），不应跨段
- 光标在最后一段时 End：到行尾
- Shift+Home/Shift+End：扩选范围正确（与非 shift 的位置一致）
- indent-aware Home 双击：第一次到第 i 段第一个非空白，第二次到第 i 段 start
- 光标不在可见区（已滚走）：回退到文档行行为
- CJK 行 + 折行：边界正确

## A3：清理 TextBuffer 的 word_wrap 死代码

**文件**：`crates/core/src/buffer/text_buffer.rs`

确认无任何调用方依赖后删除：
- 字段：`word_wrap_enabled`、`word_wrap_column`、`width`
- 方法：`set_word_wrap()`、`set_width()`、`is_word_wrap_enabled()`
- `reflow_internal()` 中的 word wrap 分支（保留按 `\n` 分行的逻辑）
- `measure_forward()` 中的 `word_wrap_column > 0` 分支
- 若 `Cursor.visual_pos` 始终 == `logical_pos`，合并字段

**保留**：`crates/core/src/unicode/measurement.rs` 的 `with_word_wrap_column` —— 这是底层度量原语，未来如果做 Path B 还会复用。

**调用方清理**：
- `crates/app/src/settings.rs:56,129,177` 的 `set_word_wrap` 调用
- `crates/app/src/commands.rs:524,561` 的 `set_word_wrap(true)` 调用
- A2 已经把 `is_word_wrap_enabled()` 的两处死分支删掉

**验收**：`cargo build && cargo test` 全绿。

---

# Path B：结构性重构（暂缓，待触发条件）

**触发条件**（满足任一才启动）：
1. 需要支持 inlay hint / inline diagnostics / virtual lines
2. 需要支持代码折叠（fold ranges）
3. WrapIndex 全量重算成为性能瓶颈（resize / 切换 viewport_width 时卡顿明显）

**核心思路**：把现有 `WrapIndex + AdvanceCacheEntry + LineCache` 收敛为一个统一的 `WrapMap`（不取名 DisplayMap，避免与 Zed 的 6 层管线总和概念撞车），并支持：
- 增量重算（仅重算 dirty 的行，类似 Zed 的 `pending_edits` + `interpolate`）
- 类型安全的 `BufferPoint` / `DisplayPoint` 坐标转换
- 后续在 `WrapMap` 上层叠加 `InlayMap` / `FoldMap`

**关键设计要点**（不展开详细 step，留给触发后再写）：
1. `WrapMap` 放在 app crate（依赖 shaping），core 保持纯文本
2. 数据结构：在现有 `WrapIndex` 的 segment tree 上增加按 doc_line 索引的 `Vec<DisplayLine>`，DisplayLine 含 `visual_lines + cluster_positions`
3. 替换 `AdvanceCacheEntry` 和 `LineCache` 为 `DisplayLine` 视图
4. 不引入 `LayoutSource` trait——只有一个实现的抽象没意义
5. 双跑验证：因为新算法（词边界）与旧算法（硬断行）行数必然不同，**不能用 `assert_eq!`**；改为「采样 100 个长行，离线 diff 视觉行边界，人工 review」

---

## 与 plans_cold_startup_perf.md 的关系

- Path A 不影响冷启动路径：`shape_cache` / `wrap_cache` key 不变
- A1 修改 `compute_visual_lines` 内部算法，**首次启动会让 wrap_cache 命中率降为 0**（key 含 content_hash，但 cache 是进程内的；冷启动本来就是空 cache，所以无影响）
- 若 Path B 启动，需要重新评估 wrap_cache 是否仍有意义（增量重算可能让它变得多余）

## 阶段切分

| 阶段 | 内容 | 工作量 | 风险 |
|------|------|--------|------|
| A1 | 词边界折行算法 + 单测 | ~80 行 | 低（纯函数） |
| A2 | 4 个命令走 advance_cache + 测试 | ~50 行 | 中（要覆盖光标不在可见区的回退） |
| A3 | 删 TextBuffer 死代码 | ~30 行删除 | 低 |
| B  | 结构性重构 | 暂不估 | 高（核心管线改写） |

A1/A2/A3 应在 3 个独立 PR 中提交，每个都可独立 build & test。
