# Visual / Doc Line 方案设计

## 1. 背景

### 1.1 当前状态

当前模型：**1 doc line = 1 visual line**，无区分。

```
Viewport
├── scroll_line: usize      // 文档行号（0-based）
├── visible_rows: usize     // 屏幕能显示几行
└── total_lines: usize      // 文档总行数

DocumentView
├── buffer: GapBuffer
├── line_offsets: Vec<usize>    // 每行起始 byte offset
├── line_lengths: Vec<usize>    // 每行 byte 长度（不含 \n）
└── viewport: Viewport
```

渲染流程：`visible_range()` → doc line 50..100 → `visible_line(i)` → `shape()` → `draw()`

### 1.2 问题

1. **长行溢出**：shape 出来的 glyph 总宽超过屏幕宽度时，直接画出屏幕被 GPU 裁剪
2. **变宽字体不支持**：当前 `PixelAdvance` 是假的（UCD 表 × em_width），对变宽字体错误
3. **字号/行高硬编码**：`FONT_SIZE=14.0`, `LINE_HEIGHT=20.0` 是常量

### 1.3 目标

- 支持 word wrap（长行自动换行）
- 支持变宽字体（真字体度量）
- 字号/行高/字体可配置
- 性能达标：60fps，每帧 < 16ms

---

## 2. 核心概念

### 2.1 Doc Line vs Visual Line

```
Doc line 5: "The quick brown fox jumps over the lazy dog"
  ↓ word wrap (viewport width = 300px)
Visual line 12: "The quick brown fox jumps"
Visual line 13: "over the lazy dog"
```

- **Doc line**：文档中的逻辑行（以 `\n` 分隔）
- **Visual line**：屏幕上显示的一行（可能被 word wrap 拆分）

### 2.2 Cursor 复用

直接复用 `core::unicode::measurement::Cursor`，无新类型：

```rust
pub struct Cursor {
    pub offset: usize,        // byte offset in buffer
    pub logical_pos: Point,   // (col, doc_line)，与 word wrap 无关
    pub visual_pos: Point,    // (col, visual_line)，受 word wrap 影响
    pub column: CoordType,    // 视觉列（用于 tab 对齐，与 wrap 无关）
    pub wrap_opp: bool,       // 当前是否站在一个 wrap 机会上
}
```

新增 `Viewport.top_cursor: Cursor` 表示视口顶部位置。其余地方按现有语义使用。

### 2.3 原版 edit 的设计

原版 edit 是终端编辑器，用 `MeasurementConfig` 处理 word wrap：

```rust
// TextBuffer
word_wrap_column: CoordType    // 终端列数 = text_width()
stats.visual_lines: usize      // word wrap 后的总 visual 行数

// Navigation
goto_visual(Point{x, y}) → Cursor
  // 内部用 MeasurementConfig::with_word_wrap_column(word_wrap_column)
  // 每次都从 cursor.offset 之后向前走
```

关键：**measurement 单向只能 forward**。`measure_forward` 不能回退（measurement.rs:240-244 直接 `if cursor >= target { return cursor }`）。这一点决定了我们的 scroll_up 策略（见 §3.3）。

原版用**贪心算法**（每行尽可能多放字符），不是 Knuth-Plass（全局最优）。对代码编辑器够用。

### 2.4 断行规则

原版 edit 的 `MeasurementConfig` 已经处理了 Unicode Line Break：

```
CJK：任意两个汉字之间可断行
Emoji：ZWJ 序列（👨‍👩‍👧）不可断行
拉丁：只在空格/连字符处断行
```

我们的 `PixelAdvance` 只替换宽度来源，断行逻辑不变。

---

## 3. 设计方案

### 3.1 Settings 结构体

```rust
pub struct Settings {
    pub font_family: String,      // "Menlo", "SF Mono", "Helvetica"...
    pub font_size: f32,           // 14.0
    pub line_height: f32,         // 20.0 (通常 = font_size * 1.4)
    pub tab_width: usize,         // 4
    pub word_wrap: bool,          // true
    pub version: u64,             // 每次变更递增，用于缓存失效
}
```

Settings 变更的级联效应：

```
Settings 变更（version++）
  ├→ Shaper 重建（FontSystem 换字体）
  ├→ GraphemeAdvanceCache 自然淘汰（key 含 version）
  ├→ shape_cache 自然淘汰（key 含 version）
  ├→ GlyphAtlas 清空（glyph_id 变了）
  └→ Viewport 重算 visible_rows + 重新 measurement top_cursor
```

**用版本号而非清空**：cache key 都含 `settings_version`，旧缓存自然淘汰，不需要主动清空。

### 3.2 真 PixelAdvance（基于 cosmic-text）

#### 3.2.1 Trait 接口改造

当前 `GraphemeAdvance` trait 定义（measurement.rs:70-84）：

```rust
pub trait GraphemeAdvance: Clone {
    fn advance(&self, props: usize) -> CoordType;     // 入参是 UCD props（per-char）
    fn tab_advance(&self, current_column, tab_size) -> CoordType;
    fn clamp_cluster_width(&self, width) -> CoordType;
}
```

`measure_forward` 内层循环对 grapheme cluster 内每个 char 累加 `advance(props)`，再用 `clamp_cluster_width` 收尾。这种 per-char 模式适合终端（每 char 1 列或 2 列），但 cosmic-text 的 advance 是**整个 grapheme** 粒度——拿不到 per-char 子 advance。

**改造方案：新增 cluster-level 钩子，保留 per-char 路径作为 fallback**

```rust
pub trait GraphemeAdvance: Clone {
    fn advance(&self, props: usize) -> CoordType;
    fn tab_advance(&self, current_column: CoordType, tab_size: CoordType) -> CoordType;
    fn clamp_cluster_width(&self, width: CoordType) -> CoordType;

    /// Optional: compute advance for a complete grapheme cluster as raw bytes.
    /// Pixel impl uses this to query cosmic-text once per cluster.
    /// Terminal impl returns None → fallback to per-char `advance(props)`.
    fn cluster_advance(&self, _bytes: &[u8]) -> Option<CoordType> {
        None
    }
}
```

`measure_forward` 内层循环改为（伪代码）：

```rust
let cluster_start = offset;
// ...内部 char 循环结束，已知 cluster 范围 [cluster_start, offset_next_cluster)...
let width = if let Some(w) = self.advance.cluster_advance(
    self.buffer.read_range(cluster_start..offset_next_cluster)
) {
    w   // pixel 路径：一次性查 cosmic-text
} else {
    // terminal 路径：保留原 per-char 累加 + clamp 逻辑
    self.advance.clamp_cluster_width(accumulated_width)
};
```

收益：
- 终端实现零行为变化，原 measurement 测试集照过
- 像素实现绕开 per-char 假设，直接用 cosmic-text 真度量

代价：内层循环要把 cluster 字节范围吐出来。`buffer.read_range(start..end)` 不存在，要走 `read_forward(start)` + 截断到 `end - start`，跨 chunk 时落到一个临时 `Vec<u8>`（大多数 cluster < 16 字节，可栈上 SmallVec）。

#### 3.2.2 PixelAdvance 实现

```rust
pub struct PixelAdvance {
    /// 通过闭包注入，避免 unsafe；闭包内部持有 Rc<RefCell<Shaper>>
    lookup: Rc<dyn Fn(&[u8]) -> CoordType>,
    em_width: CoordType,    // 用于 tab_advance fallback
}

impl GraphemeAdvance for PixelAdvance {
    fn advance(&self, props: usize) -> CoordType {
        // Fallback 路径，正常不会走到（cluster_advance 总是返回 Some）
        let w = ucd_grapheme_cluster_character_width(props, 1);
        (w as CoordType) * self.em_width
    }

    fn tab_advance(&self, current_column: CoordType, tab_size: CoordType) -> CoordType {
        let tab_width = tab_size * self.em_width;
        tab_width - (current_column % tab_width)
    }

    fn clamp_cluster_width(&self, width: CoordType) -> CoordType {
        width   // pixel 不夹断
    }

    fn cluster_advance(&self, bytes: &[u8]) -> Option<CoordType> {
        Some((self.lookup)(bytes))
    }
}
```

#### 3.2.3 Cache key 改造（避免分配）

当前 `GraphemeAdvanceCache::get` 每次 `to_string()`（shaping/src/lib.rs:82）：

```rust
let key = (grapheme.to_string(), (font_size * 64.0).round() as u32);  // ← 30-50ns 分配
```

改为零分配查询：

```rust
struct CacheKey {
    grapheme: SmolStr,    // < 23 字节内联
    font_size_q: u32,
    settings_version: u64,
}

impl GraphemeAdvanceCache {
    fn get(&mut self, grapheme: &str, font_size: f32, version: u64) -> Option<CoordType> {
        // 用 raw_entry API 按 &str 哈希查询，命中后无分配
    }
}
```

性能目标：缓存命中 < 30ns/grapheme（原文档写的 15ns 不可达，下调）。Miss 仍然要 cosmic-text shape，~1µs/grapheme。

### 3.3 Viewport 改造：相对导航

**核心问题**：`goto_visual(absolute_line)` 是 O(N)——导航到第 50000 行要走 50000 行。滚动到底部需要 100ms。

**修复**：Viewport 记住当前 Cursor 位置，滚动时**相对前进/后退**。

```rust
pub struct Viewport {
    /// 当前 viewport 顶部的 Cursor
    pub top_cursor: Cursor,
    /// 屏幕能显示几行 visual line
    pub visible_rows: usize,
    /// 文档总 doc line 数
    pub total_doc_lines: usize,
    /// word wrap 后的总 visual line 数（后台计算，见 §3.4）
    pub total_visual_lines: Option<usize>,
}
```

#### 3.3.1 scroll_down：直接 forward

```rust
fn scroll_down(&mut self, delta: usize, doc: &dyn ReadableDocument, advance: A) {
    // 从当前 top_cursor 前进 delta 个 visual line
    let target = Point { x: 0, y: self.top_cursor.visual_pos.y + delta as i32 };
    self.top_cursor = MeasurementConfig::with_advance(doc, advance)
        .with_word_wrap_column(viewport_pixel_width)
        .with_cursor(self.top_cursor)
        .goto_visual(target);
    self.clamp();
}
```

性能：scroll_down 1 行 ≈ 走当前 visual line 的字节数 × ~10ns/grapheme ≈ **几 µs**。

#### 3.3.2 scroll_up：锚到 doc line 起点重 forward

`measure_forward` 不能后退（measurement.rs:240）。要倒退 visual line，必须从某个已知更早的 cursor 向前重走。

```rust
fn scroll_up(&mut self, delta: usize, doc: &dyn ReadableDocument, advance: A,
             line_offsets: &[usize]) {
    let cur_doc_line = self.top_cursor.logical_pos.y as usize;
    let cur_vis_x = self.top_cursor.visual_pos.x;

    // Step 1: 找一个起锚 doc line —— 从当前 doc line 起，往前走 doc line
    // 直到累计 visual line 数 >= delta，再回到该 doc line 起点重 forward。
    //
    // 简化策略：直接锚到当前 doc line 起点，先尝试在本 doc 行内倒退 visual line；
    // 不够则锚到上一 doc line 起点，继续。
    let mut anchor_doc_line = cur_doc_line;
    let mut remaining = delta;

    // 当前 doc line 内有多少 visual line 在 top_cursor 之上？= top_cursor.visual_pos.y - 该行第一个 visual_y
    // 简单估算：如果 cur_vis_x > 0 或 top_cursor 不在 doc line 起点，本行内还有 visual line 可退
    // 但精确值要重 forward 才知道；直接锚到上一 doc line 起点保证够用：
    while remaining > 0 && anchor_doc_line > 0 {
        anchor_doc_line -= 1;
        // 估算该 doc line 占的 visual line 数（O(行内字节)）
        let segs = measure_doc_line_visual_segments(doc, advance.clone(),
                                                     line_offsets[anchor_doc_line], ...);
        if segs >= remaining {
            break;
        }
        remaining -= segs;
    }

    // Step 2: 从 anchor doc line 起点起一个新 cursor，forward 到目标 visual line
    let anchor = Cursor {
        offset: line_offsets[anchor_doc_line],
        logical_pos: Point { x: 0, y: anchor_doc_line as i32 },
        visual_pos: Point { x: 0, y: 0 },   // visual_y 在新一轮 measurement 内重新累计
        column: 0,
        wrap_opp: false,
    };
    let target_visual_y_relative = /* 计算从 anchor 到目标的相对 visual_y */;
    self.top_cursor = MeasurementConfig::with_advance(doc, advance)
        .with_word_wrap_column(viewport_pixel_width)
        .with_cursor(anchor)
        .goto_visual(Point { x: 0, y: target_visual_y_relative });
    // 修正 visual_pos.y：加上 anchor 之前的累计 visual line 数（从外部 total 维持）
    self.top_cursor.visual_pos.y += /* 累计偏移 */;
    self.clamp();
}
```

**性能上界**：scroll_up `delta` 行最坏情况 = forward 走 `delta + 当前 doc line 剩余` 个 visual line。

- 典型代码（每 doc line ≤ 1 个 visual line）：~delta µs。
- 极端长行（单 doc line wrap 出 1000 visual line，向上滚 1 行）：要重 forward 整个 doc line，~1ms。

`measure_doc_line_visual_segments` 为了估算 doc line 占多少 visual line 也要 forward 走全行——所以"估行数"和"目标 forward"可以合并成一次走。实现时用单次 forward 走到 doc line 末尾或 visual_y 超出目标即停。

**注**：此处不缓存任何东西（参见 §2.3，原版 edit 的取舍）。如果实测向上滚不达标，再考虑加 doc line → visual line 数的稀疏索引（每 1024 行一个采样点）。

### 3.4 total_visual_lines 的处理

滚动条需要总 visual line 数。

**策略：后台 worker 渐进计算 + UI 阻塞期间用 doc line 估算**

```
打开文件
  ├→ 主线程：total_visual_lines = None
  ├→ 主线程立刻可滚动（用 total_doc_lines 估算滚动条位置）
  └→ 后台 worker：从 doc line 0 开始，每 1k 行 yield 一次
       每次 yield 把累计 visual line 数发给主线程
       全部走完后 total_visual_lines = Some(实际值)

编辑事件
  ├→ 当前 doc line 重 measurement（O(行内字节)）
  ├→ 用增量更新 total_visual_lines（如果已算完）
  └→ 如果后台 worker 还在跑，把它停掉重启（编辑期间不用精确值）
```

收益：
- UI 永不阻塞
- 大文件首次打开几秒后滚动条精确
- 编辑频繁的文件以 doc line 估算（用户视觉容忍）

### 3.5 渲染流程改造

```
旧流程：
  visible_range() → doc line 50..100
  → visible_line(i) → &[u8]
  → shape() → render

新流程：
  cursor = viewport.top_cursor
  for i in 0..visible_rows:
    // 从 cursor 取当前 visual line 的字节范围 + 走到下一 visual line 的 cursor
    (byte_range, next_cursor) = measurement.measure_visual_line(cursor)

    bytes = buffer.read_range(byte_range)   // 跨 chunk 时复制到 scratch
    shaped = shape(bytes)                    // shape_cache 命中时跳过
    render(shaped, y_pos = i * line_height)

    cursor = next_cursor
```

关键：**不需要 goto_visual(absolute)**。从 top_cursor 开始，逐 visual line 前进。每行 measurement = O(行内字节)。

### 3.6 Cache 设计

| Cache | Key | Value | 失效条件 |
|---|---|---|---|
| `GraphemeAdvanceCache` | `(grapheme, font_size_q, settings_version)` | `CoordType` advance | settings_version 变 |
| `shape_cache` | `(line_offset, line_byte_len, settings_version)` | `ShapedRun` | 内容变更、settings_version 变 |

**不缓存 visual line 边界**。原因：

1. 原版 edit 不缓存（实时计算够快）
2. 大文件 1024 行 LRU 命中率不可控（用户跨区域滚动直接 miss）
3. 增量编辑后失效 invalidation 复杂
4. 真测下来 measurement 50 行 < 200µs，单帧预算 16ms 内充裕

如未来 perf bench 不达标，再加每 1024 doc line 一个采样点的稀疏索引。

### 3.7 增量编辑的 reflow

> 注：编辑能力在 plans 阶段 6 才到位。本节只列出**接口约定**，实际实现 follow 阶段 6。

当前 `DocumentView.line_offsets/line_lengths` 不是为编辑设计的——单字符插入要 O(N) 改写所有后续 offset。这一层会在 plans 阶段 6 替换为 edit 上游的 `TextBuffer`（带 `LineEndingState` 增量维护）。本设计暂不承诺"1µs 单字符 reflow"。

阶段 6 改造完成后，输入流程是：

```rust
fn on_char_insert(&mut self, cursor: &Cursor, byte: &[u8]) {
    // 1. 插入字符到 GapBuffer（gap 在 cursor 处时 O(1)）
    self.buffer.replace(cursor.offset..cursor.offset, byte);

    // 2. TextBuffer 内部增量更新行索引（O(改动局部)）
    self.text_buffer.notify_edit(cursor.offset, byte.len());

    // 3. 当前 visual line 的 shape_cache invalidate
    self.shape_cache.invalidate_line(cursor.logical_pos.y);

    // 4. total_visual_lines 增量修正（可延迟到下次空闲）
}
```

**性能预期**（阶段 6 时验收）：单字符插入 < 1ms（含 reflow + 重 shape 当前行）。本节不做硬承诺。

### 3.8 为什么不用 cosmic-text 自带 Wrap

cosmic-text `Buffer::set_size` + `Wrap::Word` 也能 word wrap，layout_runs 直接吐 visual line。我们仍然走自家 `MeasurementConfig` 的原因：

1. **测试基线复用**：`TerminalAdvance` 通过的 100+ 测试是 word wrap 行为基线。换 cosmic-text 后无法回归终端行为。
2. **整数定点**：measurement 全 i32 算术（`CoordType`），无浮点累计误差。cosmic-text 内部 f32。
3. **流式 cursor 模型**：`MeasurementConfig` 支持任意起点 forward，天然适合 viewport 相对导航。cosmic-text `Buffer` 是整段输入整段输出。
4. **内存控制**：cosmic-text 的 `Buffer` 持有 shape 结果，大文件全量 layout 内存爆。我们只 measure 不持久化。

代价：维护两套 wrap 逻辑（measurement 算 break 点、cosmic-text 算 glyph 几何）。视为可接受。

### 3.9 行尾归一化

`measure_forward` 仅识别 `\n`（`ucd_linefeed_properties()`）作为硬换行。CR-only（老 Mac 行尾，plans §9 列为兼容样本）和 CRLF 中的 `\r` 在 measurement 中被当成普通字符，会出现：

- CR-only 文件：整个文件被当成一个 doc line，word wrap 后视觉行数等于全文 grapheme 数 / 行宽。功能不崩，但语义错。
- CRLF 文件：每行末尾 `\r` 占一个零宽位置（UCD 把 CR 标为 control char，width = 0），visual 上表现为 LF 行尾——正确。

**策略：加载时归一化为 LF，保存时还原**

```
load_file():
  detect_eol(buffer) → Lf | Crlf | Cr | Mixed
  if not Lf:
    rewrite buffer in place: Cr/Crlf → Lf
  remember original_eol 用于保存

save_file():
  if original_eol != Lf:
    rewrite outgoing bytes back
```

这样 measurement 永远只看 LF，无 CR 兼容性坑。归一化逻辑放在 plans 阶段 5（只读显示）的文件加载路径里。

---

## 4. 性能分析

### 4.1 各环节耗时（更新后的预算）

| 环节 | 耗时/帧 | 说明 |
|---|---|---|
| scroll_down 1 行 | ~5µs | forward 当前 visual line 的字节数 |
| scroll_up 1 行（典型） | ~10µs | 同上 + 锚到上一 doc line 重 forward |
| scroll_up 1 行（极端长行） | ~1ms | 单 doc line wrap 出千行的最坏情况 |
| measurement × 50 行 | ~200µs | grapheme cache 命中 30ns，假设 50 行 × 80 grapheme |
| `shape()` × 50 行 ASCII | ~1ms | shape_cache 命中后 ~0；miss 时 cosmic-text 实测 ~20µs/行 |
| `shape()` × 50 行 CJK 混排 | ~3ms | CJK shape 慢于 ASCII |
| `generate_vertices` | ~100µs | 顶点生成 |
| GPU render | ~1ms | wgpu 提交 |
| **总计（典型 ASCII）** | **~2.3ms** | 14% 的 16ms 预算 |
| **总计（CJK 混排首帧）** | **~4.5ms** | 28%，仍有余 |

### 4.2 与当前实现对比

| | 当前（无 word wrap） | 新方案（有 word wrap） |
|---|---|---|
| 滚动 | 0（直接算 doc line） | 5µs（相对导航） |
| measurement | 0 | 200µs |
| shape | 1ms | 1ms（cache 命中） |
| render | 1ms | 1ms |
| **总计** | **~2ms** | **~2.3ms** |

增量 ~300µs（1.9%），可接受。

### 4.3 性能验收硬门槛

| 场景 | 阈值 | 说明 |
|---|---|---|
| `bench_scroll_down_60s_60fps` | 丢帧 < 1% | 模拟连续滚动 |
| `bench_scroll_up_60s_60fps` | 丢帧 < 1% | 反向滚动 |
| `bench_shape_visible_50_lines_ascii` | < 2ms | shape miss 路径 |
| `bench_shape_visible_50_lines_cjk_mixed` | < 4ms | CJK 真实预算 |
| `bench_grapheme_advance_cache_hit` | < 30ns | 零分配查询 |
| `bench_long_line_1mb_first_paint` | < 50ms | 1MB 单行首屏 |
| `bench_settings_change_to_first_paint` | < 100ms | 字号切换响应 |

### 4.4 边界情况

| 场景 | 耗时 | 说明 |
|---|---|---|
| 1MB 单行首次显示 | ~50ms | measurement 走全行（grapheme cache 冷） |
| 1MB 单行后续滚动 | ~1ms/帧 | 向上滚要回到行首重 forward |
| 100k 行 × 80 字符 | 正常 | 每帧只 measurement 50 行 |
| 变宽字体 "WWWWWW..." | 正常 | grapheme cache 命中率高 |
| Settings 频繁变更 | 慢 | 每次 version++，所有 cache 失效，下一帧 cold |

---

## 5. 实现顺序

### Phase 1：Settings 结构体（~1h）

- 抽取 `Settings` 到独立模块
- `FONT_SIZE` / `LINE_HEIGHT` 从常量改 `Settings` 字段
- Shaper、Viewport、DocumentView 接受 `&Settings`
- 纯重构，不加功能

**涉及文件**：`app.rs`, `viewport.rs`, `document_view.rs`, `shaping/lib.rs`, 新增 `settings.rs`

### Phase 2：GraphemeAdvance trait 增强 + 真 PixelAdvance（~3h）

- `GraphemeAdvance` 新增 `cluster_advance(bytes) -> Option<CoordType>`
- `measure_forward` 内层循环吐出 cluster 字节范围，优先调用 `cluster_advance`
- `TerminalAdvance::cluster_advance` 返回 `None`（保留 per-char 路径）
- `PixelAdvance` 接入 `Shaper::grapheme_advance`（闭包注入，不用 unsafe）
- `GraphemeAdvanceCache` key 改 SmolStr + settings_version，零分配查询
- 验证：`TerminalAdvance` 测试集逐字段对齐原版；`PixelAdvance` 在 Helvetica 下 measurement 正确

**涉及文件**：`core/unicode/measurement.rs`, `shaping/lib.rs`

**验收**：
- 阶段 1 全部 measurement 测试用 `TerminalAdvance` 跑过，0 失败
- `bench_grapheme_advance_cache_hit < 30ns`
- 新增 `pixel_advance_variable_width_font` 测试

### Phase 3：行尾归一化 + Viewport 相对导航（~3h）

- 加载时归一化 CR/CRLF → LF，保存时还原
- Viewport 改为持有 `top_cursor: Cursor`
- `scroll_down` 用相对 forward
- `scroll_up` 用 anchor doc line 重 forward
- 渲染流程用 `measure_visual_line` 逐行前进
- 手动测试：长行自动换行；CR-only 文件正常显示

**涉及文件**：`viewport.rs`, `document_view.rs`, `core/file.rs`, `app.rs`

**验收**：
- `bench_scroll_down_60fps` / `bench_scroll_up_60fps` 双绿
- `assets/samples/small_cr_only.txt` 显示行数正确
- `assets/samples/long_line_1mb.txt` 不卡

### Phase 4：total_visual_lines 后台计算（~2h）

- 后台 worker thread：分批 measurement，每 1k 行 yield
- 主线程在算完前用 `total_doc_lines` 估算滚动条
- 编辑触发时停掉旧 worker，重启
- 性能基准

**涉及文件**：`document_view.rs`, `app.rs`

**验收**：
- 50MB 文件首次加载后 5s 内 `total_visual_lines` 算完
- 滚动条在算完前位置近似但不跳

### Phase 5（延迟到 plans 阶段 6）：增量 reflow

- 等 `TextBuffer` + `LineEndingState` 替换 `line_offsets` 之后
- 输入时只重 measurement 当前 visual line
- shape_cache 按 visual line 粒度失效
- `total_visual_lines` 增量修正

---

## 6. 验收标准

### 功能验收

- [ ] 长行自动换行，不超出屏幕
- [ ] 变宽字体（Helvetica）word wrap 正确
- [ ] 字号 8/14/72 都正常
- [ ] resize 后 word wrap 重算
- [ ] 滚动顺滑（visual line 粒度）
- [ ] CJK / Emoji 断行正确
- [ ] CR-only / CRLF / 混排行尾正确

### 性能验收

- [ ] 60fps 持续滚动不掉帧（上下双向）
- [ ] scroll_down 1 行 < 10µs
- [ ] scroll_up 1 行（典型） < 20µs
- [ ] grapheme advance cache 命中 < 30ns
- [ ] shape 50 行 CJK 混排 < 4ms
- [ ] 50MB 文件 word wrap 后 RSS < 200MB
- [ ] Settings 变更后首帧 < 100ms
- [ ] 50MB 文件 total_visual_lines 后台算完 < 5s

### 边界验收

- [ ] 1MB 单行首屏 < 50ms，后续滚动不卡
- [ ] 空文件正常
- [ ] CRLF/LF/CR 混排正常
- [ ] 非法 UTF-8 不崩
- [ ] ZWJ emoji 不被断行拆开
- [ ] viewport 高度 < 1 行时不崩
