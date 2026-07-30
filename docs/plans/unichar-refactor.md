# 改造方案：统一到 Grapheme 坐标系

## 1. 背景与问题

当前编辑器存在两套平行的坐标系：

| 层 | 坐标类型 | 用途 |
|---|---|---|
| 预览选择 `selection::hit_test` | `ViewPos { flat_line_idx, char_pos }` | 渲染文本选择 |
| 编辑光标 `mouse::hit_test` | `byte_offset: usize` | 鼠标点击定位光标 |
| 光标状态 `Cursor` | `ByteIndex` + `LogicalPoint { line, grapheme }` | 双轨并存 |
| 缓冲区 API | `cursor_move_to_byte(ByteIndex)` | 主入口 |

**核心矛盾**：`Cursor` 已经同时维护了 byte offset 和 grapheme 位置（`LogicalPoint`），但对外 API 以 byte 为主。上层（app 层）被迫在 byte 坐标系里工作，而 byte 偏移对用户无意义——用户看到的是"第 3 个字符"，不是"第 7 个字节"。

更关键的是，**API 不干净导致容易写出错误代码**：上层的 197 处 `ByteIndex` 引用分散在 21 个文件中，每次新增功能都要在 byte 和 grapheme 之间做心智转换，是 bug 的温床。

### 现有基础设施

以下 grapheme 相关设施**已经存在**，无需重写：

- `CursorNav::goto_logical(LogicalPoint)` — 按 grapheme cluster 导航（`cursor_nav.rs:231`）
- `CursorNav::measure_forward` — 核心引擎，同时计算 byte offset 和 grapheme 位置（`cursor_nav.rs:256`）
- `ucd_grapheme_cluster_joins` / `ucd_grapheme_cluster_joins_done` — UAX #29 边界判定（`tables.rs`）
- `LogicalPoint { line, grapheme }` — 已定义的逻辑位置类型（`indices.rs:81`）

## 2. 改造目标

将上层 API 的坐标系从 **byte offset** 统一为 **grapheme**（UAX #29 extended grapheme cluster），使：

1. 鼠标点击返回 grapheme 位置而非 byte 位置
2. 光标移动 API 以 grapheme 为主入口
3. 上层不再直接接触 `ByteIndex`，byte 转换下沉到 buffer 层
4. 内部实现（gap buffer、shaper、SIMD）仍以字节为单位，不变

## 3. 技术对比

### 3.1 数据结构变更

#### LineIndex（`crates/app/src/line_index.rs`）

```rust
// 改造前
pub(crate) struct LineIndex {
    pub(crate) offsets: Vec<usize>,   // 每行 byte offset
    pub(crate) lengths: Vec<usize>,   // 每行 byte 长度
}

// 改造后
pub(crate) struct LineIndex {
    pub(crate) offsets: Vec<usize>,          // 每行 byte offset（不变）
    pub(crate) lengths: Vec<usize>,          // 每行 byte 长度（不变）
    pub(crate) grapheme_counts: Vec<u32>,    // 新增：每行 grapheme cluster 数
}
```

**`grapheme_counts[i]` 语义**：第 `i` 行的 **local** grapheme 数量（不含换行符）。上层计算全局偏移时通过 `grapheme_counts[0..line]` 求和 + 行内偏移得出。

**增量更新**：编辑后通过 `rebuild_from(start_byte)` 重扫受影响的尾部行，grapheme_counts 同步截断并重建，与 offsets/lengths 保持同一生命周期。

**内存开销**：

| 文件规模 | 当前 LineIndex | 新增 `grapheme_counts` | 增幅 |
|---|---|---|---|
| 1 万行 | 160 KB | +40 KB | +25% |
| 10 万行 | 1.6 MB | +400 KB | +25% |
| 100 万行 | 16 MB | +4 MB | +25% |

> 每行 4 字节（`u32`，最大支持 ~40 亿 grapheme/行，远超实际需求）。

#### AdvanceCacheEntry（`crates/ui/src/render_geom.rs`）

```rust
// 改造前
pub struct AdvanceCacheEntry {
    pub doc_line: usize,
    pub vl_byte_start: usize,
    pub clusters: Vec<(usize, f32)>,  // (cluster_end_byte, pixel_x)
}

// 改造后
pub struct AdvanceCacheEntry {
    pub doc_line: usize,
    pub vl_byte_start: usize,
    pub vl_grapheme_start: usize,                       // 新增：视觉行起始 grapheme（行内偏移）
    pub clusters: Vec<(usize, f32, u32)>,               // (cluster_end_byte, pixel_x, grapheme_count)
}
```

**`grapheme_count` 字段说明**：shaper 输出的 cluster 边界以 byte 为单位，一个 cluster 可能对应多个 grapheme（如连字 "fi"：1 个 cluster，2 个 grapheme；表情 ZWJ 序列：多个 cluster 组成 1 个 grapheme）。存储 `grapheme_count`（而非累计索引）使 hit_test 可以逐 cluster 累加计算当前位置的 grapheme 偏移。

**内存开销**（视口 60 行，行均 100 cluster）：

| | 当前 | 新增 | 增幅 |
|---|---|---|---|
| clusters | 60 × 100 × 16B = 96 KB | +48 KB（tuple 16→24B 因对齐） | +50% |

> 仅视口范围内的 cache，非全文。

#### Cursor（不变）

```rust
// 不变 — grapheme 字段名保留
pub struct Cursor {
    pub offset: ByteIndex,        // 内部仍用 byte
    pub logical_pos: LogicalPoint, // .grapheme 语义不变
    pub visual_pos: VisualPoint,
    pub column: CoordType,
}
```

### 3.2 性能对比

#### 文件加载

| 操作 | 改造前 | 改造后 | 差异 |
|---|---|---|---|
| 扫描换行符建行表 | O(字节数) | O(字节数)，同时计 grapheme | 同一循环，零额外遍历 |
| 10 万行 / 500 万字符 | ~100ms | ~120ms（+4ns/字节 is_continuation 判断） | **+20%** |
| 100 万行 / 5000 万字符 | ~1s | ~1.2s | **+20%** |

> 加载是一次性操作，+20% 绝对值很小，用户无感知。实际瓶颈通常在磁盘 I/O 和 shaping。

#### Shaper 输出

```rust
// 改造后：shaper 需统计当前 cluster 包含的 grapheme 数量
// 兼容连字（如 "fi" 被塑形为 1 个 cluster，但对应 2 个 grapheme）
let grapheme_count = count_graphemes(&text[cluster_start..cluster_end]);
```

| 操作 | 开销 | 占比 |
|---|---|---|
| 每 cluster | +1ns（整数自增） | **< 0.1%**（shaping 约 30-50ns/cluster） |

#### 鼠标点击（hit_test）

```rust
// 改造前：查 cluster tuple 取 byte_end
byte_offset = vl_byte_start + cluster_end;

// 改造后：遍历 clusters，累加 grapheme_count
// 若点击落在某个 grapheme_count > 1 的 cluster 内（如连字），则按 X 坐标比例插值
grapheme_offset = vl_grapheme_start + accum_graphemes + interpolated_offset;
```

| 操作 | 改造前 | 改造后 | 差异 |
|---|---|---|---|
| 查找 | 遍历 clusters 取 byte | 遍历 clusters 累加 grapheme | 同一次遍历，多一个整数加法 | **~1%** |
| 光标设置 | `cursor_move_to_byte` | `cursor_move_to_grapheme` | 内部走同一 `measure_forward` 引擎 |

> 热路径（鼠标移动每帧触发）有微小开销增加，但在视口 ~6000 cluster 的规模下绝对值不可感知。

#### 光标移动

```rust
// 改造前
pub fn cursor_move_to_byte(&mut self, offset: ByteIndex) { ... }

// 改造后
pub fn cursor_move_to_grapheme(&mut self, line: usize, grapheme: usize) {
    // 内部调用已有的 cursor_move_to_logical
    self.cursor_move_to_logical(LogicalPoint { line, grapheme });
}
```

`cursor_move_to_logical` 和 `cursor_move_to_byte` 共享同一个 `measure_forward` 引擎，性能**完全相同**。

#### 编辑后更新

| 操作 | 改造前 | 改造后 | 差异 |
|---|---|---|---|
| 单行编辑（80 字符） | gap buffer 移动 + 重建行表尾部 | 同左 + 重数受影响行 grapheme + 更新后续行 | +320ns（80×4ns） |
| 粘贴 1000 行 | gap buffer 大块拷贝 | 同左 + 扫描粘贴文本 grapheme | +几 µs |

### 3.3 内存总览

| 场景 | 当前总内存（估算） | 新增 | 增幅 |
|---|---|---|---|
| 1 万行代码文件，视口 60 行 | ~500 KB | +88 KB（LineIndex 40K + cache 48K） | +18% |
| 10 万行代码文件，视口 60 行 | ~3 MB | +448 KB | +15% |

## 4. API 变更清单

### 4.1 核心变更

| 位置 | 改造前 | 改造后 |
|---|---|---|
| `TextBuffer::cursor_move_to_byte(ByteIndex)` | 主入口 | 保留但标记 `pub(crate)`，仅供内部 byte→grapheme 转换 |
| `TextBuffer` | — | 新增 `cursor_move_to_grapheme(line: usize, grapheme: usize)` |
| `mouse::hit_test` | 返回 `(byte_offset, doc_line, vis_line)` | 返回 `(grapheme_offset, doc_line, vis_line)` |
| `LineIndex` | `offsets + lengths` | 新增 `grapheme_counts: Vec<u32>` |
| `AdvanceCacheEntry` | `vl_byte_start + clusters: Vec<(usize, f32)>` | 新增 `vl_grapheme_start`；clusters 加 `grapheme_count` 字段 |

### 4.2 保留不动

| 组件 | 理由 |
|---|---|
| `GapBuffer` | 纯字节容器，是存储层，不暴露给上层 |
| `Cursor.offset: ByteIndex` | 内部实现细节，buffer 层需要 |
| `AdvanceCacheEntry.clusters[*].0`（cluster_end_byte） | shaper 产出的原始字节边界，hit_test 内部使用 |
| `simd::lines_fwd` / `lines_bwd` | 字节级 SIMD，不受影响 |
| `LogicalPoint` 及其 `grapheme` 字段 | 命名和语义均不变 |

## 5. 影响范围

### 5.1 文件清单

#### crates/core（底层，改动小）

| 文件 | 改动 |
|---|---|
| `buffer/text_buffer.rs` | 新增 `cursor_move_to_grapheme`；`cursor_move_to_byte` 降为 `pub(crate)` |
| `buffer/selection.rs` | 跟随新 API（`cursor_move_to_grapheme`） |
| `buffer/navigation.rs` | 无实质改动，内部仍用 byte |
| `buffer/edit.rs` | 内部仍用 byte，无实质改动 |
| `buffer/search.rs` | 跟随新 API |

#### crates/app（应用层，改动较多）

| 文件 | 改动 |
|---|---|
| `line_index.rs` | 新增 `grapheme_counts` 字段和构建逻辑；新增 `grapheme_at_line(line) -> usize` / `line_at_grapheme(offset) -> (line, local)` |
| `mouse.rs` | `hit_test` 返回 grapheme 偏移；内部 byte→grapheme 映射 |
| `document_view/mod.rs` | `cursor_move_to_byte` 调用改为 `cursor_move_to_grapheme` |
| `document_view/selection.rs` | 同上 |
| `document_view/edit.rs` | 同上 |
| `document_view/cursor.rs` | ByteIndex 直接使用迁移 |
| `cursor_motion.rs` | `ByteIndex` → grapheme |
| `commands.rs` | `ByteIndex` 直接使用迁移 |
| `workspace.rs` | `ByteIndex` 直接使用迁移 |

#### crates/ui（UI 组件库，改动小）

| 文件 | 改动 |
|---|---|
| `render_geom.rs` | `AdvanceCacheEntry` 加 `vl_grapheme_start` 和 tuple 第三字段；新增 `grapheme_to_x` / `x_to_grapheme` |

#### crates/markdown（预览层，改动极小）

| 文件 | 改动 |
|---|---|
| `selection.rs` | 跟随 `ViewPos` 字段调整 |
| `view.rs` | `hit_test_byte` 保留（WYSIWYG 编辑内部用） |

### 5.2 调用点统计

| 类别 | 数量 |
|---|---|
| `ByteIndex` 引用（全局） | 21 文件，~197 处（app 层） |
| `cursor_move_to_byte` 调用 | 28 处（其中 14 处在测试） |
| `cursor.offset` 直接访问 | 66 处 |

> 多数改动是机械替换（`ByteIndex(offset)` → `(line, grapheme)`），非逻辑变更。

## 6. 实施计划

### Phase 1：Grapheme 计数基础设施

1. **明确 grapheme 计数算法**：必须复用 `CursorNav` 已有的 `measure_forward` 中使用的 UAX #29 extended grapheme cluster 边界判定逻辑，确保 LineIndex 的 grapheme 计数与光标导航的 grapheme 移动完全一致。具体规则：
   - 使用 Extended Grapheme Cluster 边界（UAX #29 规则 GB1-GB999）
   - CRLF 序列计为 1 个 grapheme（规则 GB3）
   - Hangul LVT 音节按规则 GB12-GB13 处理
   - Emoji ZWJ 序列计为 1 个 grapheme（规则 GB11）
   - Regional Indicator 对计为 1 个 grapheme（规则 GB12-GB13）
   - **提取共享函数**：将边界判定逻辑从 `cursor_nav.rs` 中抽出一个 `pub(crate) fn count_graphemes(text: &str) -> u32`，供 LineIndex 和 shaper 路径共同调用

2. `line_index.rs`：新增 `grapheme_counts: Vec<u32>`
3. `rebuild_from` 中合并 grapheme 计数到行扫描循环
4. **增量更新策略**：
   - `grapheme_counts` 存储每行 **local** grapheme 数
   - `rebuild_from(start_byte)` 从 `start_byte` 对应的行开始重扫，与 `offsets`/`lengths` 同步截断
   - 对于仅修改单行的编辑，只需重数当前行；`offsets`/`lengths` 的同步截断逻辑自动处理后续行号偏移
5. **新增 Benchmark**：测试 50MB 大文件初始化的真实 UAX #29 解析耗时，若超 500ms 则引入延迟/分块计算
6. 新增 `grapheme_of_line(line) -> usize` 方法（返回从文档起始到该行的累积 grapheme 偏移）
7. 新增 `line_at_grapheme(offset) -> (line, line_local_grapheme)` 二分查找
8. 单元测试：覆盖纯 ASCII、CJK、Emoji ZWJ 序列、连字文本

### Phase 2：Cursor API 扩展（新旧并存）

1. `TextBuffer` 新增 `cursor_move_to_grapheme(line: usize, grapheme: usize)`
2. 实现：直接委托 `cursor_move_to_logical(LogicalPoint { line, grapheme })`
3. 此时 `cursor_move_to_byte` 保持 `pub`，新旧 API 共存
4. 单元测试

### Phase 3：AdvanceCacheEntry 扩展

1. `render_geom.rs`：`AdvanceCacheEntry` 加 `vl_grapheme_start` 和 tuple 第三字段 `grapheme_count`
2. shaper 输出路径：每个 cluster 调用 `count_graphemes()` 累加 grapheme 计数
3. 新增 `grapheme_to_x` / `x_to_grapheme` 辅助函数

### Phase 4：hit_test 改造

1. `mouse::hit_test` 返回 `(grapheme_offset, doc_line, vis_line)`（grapheme_offset 为文档级全局偏移）
2. 内部遍历 `clusters` 累加 `grapheme_count` 计算：
   - 若点击落在 `grapheme_count == 1` 的 cluster：直接取累积 grapheme 偏移
   - 若点击落在 `grapheme_count > 1` 的 cluster（如连字 "fi"）：
     - 按 X 坐标比例在 `[0, grapheme_count)` 范围内线性插值
     - 插值结果 clamp 到 `[0, grapheme_count - 1]`
     - 这是近似行为——连字是单个 glyph，无法从像素位置精确区分 'f' 和 'i' 的分界点
     - 验收标准：点击连字左半→光标在 'f' 后；点击右半→在 'i' 后。此为业界通用做法（VS Code、Zed 同）
3. 更新所有 hit_test 调用方，改为 `cursor_move_to_grapheme(line, local_grapheme)`
4. 测试：纯 ASCII 文本点击、CJK 文本点击、连字文本点击（"fi"）、Emoji 点击

### Phase 5：ByteIndex 清理

1. 逐文件替换 app 层 `cursor_move_to_byte` 调用 → `cursor_move_to_grapheme`
2. 替换 `ByteIndex(offset)` 直接构造 → 通过 `line_at_grapheme` 转换为 `LogicalPoint`
3. `cursor_move_to_byte` 降为 `pub(crate)`，仅 buffer 层内部使用
4. 清理 app 层文件中的 `use ... ByteIndex` import
5. 验收标准：`crates/app/src/` 的非测试代码中无 `ByteIndex` 直接引用
6. 全量测试

### Phase 6：清理与回归验证

1. 清理残余 `ByteIndex` 直接使用
2. 更新文档
3. `cargo fmt` + `./scripts/verify.sh`
4. 手动验收：CJK、Emoji、连字文本的光标定位和选择行为

## 7. 风险与缓解

### 7.1 Grapheme 计数与 CursorNav 不一致

| 风险 | 如果 LineIndex 的 grapheme 计数逻辑和 CursorNav 的 grapheme 移动使用了不同的边界判定，会出现"光标移到位置 X，但 LineIndex 认为位置 X 对应不同的行/偏移"，导致光标位置错误或越界 panic |
|---|---|
| 影响 | 所有非纯 ASCII 文本的光标定位 |
| **缓解** | **绝对不独立实现**。从 `cursor_nav.rs` 抽取共享函数 `pub(crate) fn count_graphemes(text: &str) -> u32`，LineIndex 和 shaper 路径均调用此函数。在 Phase 1 第一步完成，使用同一份 UAX #29 规则和同一份 `ucd_grapheme_cluster_joins` 表 |
| 验证 | Phase 1 Benchmark 中对同一文本对比 `count_graphemes()` 和 `CursorNav::measure_forward()` 的结果 |

### 7.2 grapheme_counts 与 GapBuffer 状态不同步

| 风险 | 每次编辑后 GapBuffer 内容变更，但 grapheme_counts 更新遗漏或错误，导致 LineIndex 的 grapheme 偏移与实际文本脱节。这是整个方案**最高风险项**——一旦不同步，光标定位会随机出错，且难以排查 |
|---|---|
| 影响 | 极易触发越界 panic 或光标跳变 |
| **缓解** | 
1. **复用 rebuild_from 的生命周期**：grapheme_counts 的截断/重建与 offsets/lengths 在同一函数、同一次遍历中完成，不允许独立更新路径
2. **debug_assert 校验**：每次 `cursor_move_to_grapheme` 调用时，在 debug 模式下断言 `grapheme < grapheme_counts.sum()`，捕获越界
3. **Fuzz Testing（Phase 1）**：编写 fuzz 测试，随机生成插入/删除/替换操作序列（含 CJK、Emoji），每步后断言：
   - `LineIndex::grapheme_of_line(line_count) == cursor_nav::count_graphemes(entire_text)`
   - 对随机抽样的行 `i`：`grapheme_counts[i] == count_graphemes(line_text(i))`
4. **确定性重算**：新增 `LineIndex::validate_grapheme_counts(text: &str) -> bool` 方法，在关键路径（文件保存、切换 tab）调用做完整性校验 |

### 7.3 连字（Ligature）光标定位精度

| 风险 | Shaper 将多个 grapheme 合并为一个 cluster（如 "fi"→ 连字 glyph），此时 cluster 和 grapheme 不是 1:1，光标无法在连字内部精确定位 |
|---|---|
| 影响 | 连字文本中点选时，光标可能偏一个 grapheme 位置 |
| **缓解** | 
1. `AdvanceCacheEntry.clusters` 存储 `grapheme_count`（当前 cluster 覆盖几个 grapheme），而非累计 index
2. hit_test 中：若 `grapheme_count > 1`，按 X 坐标在 cluster 宽度内的比例线性插值，映射到 `[0, grapheme_count-1]` 的整数偏移
3. **验收测试**：构造含连字文本（"firm offer"），对 'f' 和 'i' 之间的区域做高密度逐像素点击，断言光标始终落在 'f' 后或 'i' 后，不会落在非法位置
4. **行为对标**：VS Code 和 Zed 均采用此方案，用户预期一致 |

### 7.4 大文件 Parsing 延迟

| 风险 | 对于超大单行文件（如 minified JSON 数百万字符在一行），grapheme 计数可能显著增加加载耗时 |
|---|---|
| 影响 | 用户感知的开文件延迟 |
| **缓解** | 
1. Phase 1 中编写 Benchmark（50MB 单行 JSON、50MB 正常换行文本），测量实际耗时
2. 若单行耗时 >500ms：对单行采用 **lazy counting**——仅当用户滚动到该行或在该行操作时才计算 grapheme 偏移，用 `Option<u32>` 标记未计算状态
3. 若正常换行文件耗时 >500ms：考虑分块异步构建，不阻塞主线程 |

### 7.5 197 处 ByteIndex 引用的回归风险

| 风险 | 大规模 API 变更引入机械性错误或逻辑遗漏，导致编译通过但行为异常 |
|---|---|
| 影响 | 光标移动、选择、编辑等核心交互功能退化 |
| **缓解** | 
1. **Phase 2 新旧 API 共存**：先加新 API，再逐步迁移调用方。每个 Phase 单独提交，独立可测试
2. **迁移顺序**：先迁 document_view（核心路径，测试最多），再迁 commands/cursor_motion/workspace（次要路径）
3. **每迁移一个文件就跑全量测试**，不攒到最后
4. `./scripts/verify.sh` 在每个 Phase 结束时执行 |

### 7.6 回滚方案

| 风险 | 若方案上线后发现不可预期的行为退化（尤其是连字光标体验差），需要能快速回退 |
|---|---|
| **缓解** | 
1. 保留 `cursor_move_to_byte` 的 `pub(crate)` 可见性——它仍然存在，只是不对 app 层公开
2. 回退只需 2 步：(a) `cursor_move_to_byte` 恢复 `pub`；(b) hit_test 恢复返回 byte offset
3. `LineIndex.grapheme_counts` 和 `AdvanceCacheEntry` 扩展字段是无害的——即使回退也不需移除，仅不再使用
4. GitHub 上保留 pre-refactor tag |

## 8. 验收标准

1. `cargo test -p edit-plus-app --lib` 全部通过
2. `cargo test -p edit-plus-core` 全部通过
3. 鼠标点击、拖选、双击选词、三击选行行为与改造前一致
4. CJK 文本（中日韩）、Emoji（含组合 Emoji 如 🇨🇳、👨‍👩‍👧）、连字（é = e + ◌́、fi 连字）、CRLF 换行文本光标定位正确
5. `./scripts/verify.sh` 通过
6. `crates/app/src/` 的非测试公开 API 中无 `ByteIndex` 残留
7. Fuzz 测试在 CI 中运行，覆盖 ≥10000 次随机编辑操作
8. Benchmark 显示 50MB 文件加载耗时 <500ms
