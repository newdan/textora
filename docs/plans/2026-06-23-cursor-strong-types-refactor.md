# 重构 Cursor 坐标系统与强类型安全

目前 `edit+` 的底层在处理多字节字符和排版时，存在严重的**基本类型偏执（Primitive Obsession）**问题：混用 `usize` 和通用的 `Point` 结构体来表示字节、字形和视觉坐标。这导致编译器无法在编译期拦截非法的坐标跨维度计算，极易引发光标错位或越界 Panic。此外，`CursorNav::measure_forward` 的 O(N) 遍历存在较大的性能隐患。

本方案旨在彻底重构 `Cursor` 的数据类型，并为后续的高效 `LineMap` 排版引擎铺平道路。

## 破坏性变更说明

本次重构将修改 `crates/core/src/unicode/cursor_nav.rs` 及所有依赖 `Cursor` / `CursorNav` 的模块（包括 `app` 层和 `buffer/navigation.rs`）。所有原先传递 `usize` 字节偏移量或 `Point` 的地方都需要显式地包裹为强类型。

## 当前代码现状分析

### 核心类型（`crates/core/src/helpers.rs`）

```rust
pub type CoordType = isize;  // 列坐标、像素宽度、tab_size 等全用这一个类型

pub struct Point {
    pub x: CoordType,  // 既表示 grapheme index，又表示 visual column
    pub y: CoordType,  // 既表示 logical line，又表示 visual row
}
```

### Cursor 结构体（`crates/core/src/unicode/cursor_nav.rs:12-27`）

```rust
pub struct Cursor {
    pub offset: usize,        // 字节偏移 — 裸 usize
    pub logical_pos: Point,   // (grapheme clusters, lines)
    pub visual_pos: Point,    // (columns, rows)
    pub column: CoordType,    // ≡ visual_pos.x，仅为 tab 计算便利而冗余存储
}
```

### CursorNav API（`cursor_nav.rs:219-243`）

```rust
pub fn goto_offset(&mut self, offset: usize) -> Cursor           // 裸 usize
pub fn goto_logical(&mut self, logical_target: Point) -> Cursor  // 裸 Point
pub fn goto_visual(&mut self, visual_target: Point) -> Cursor    // 裸 Point
```

### TextBuffer 公开 API（`crates/core/src/buffer/text_buffer.rs`）

```rust
pub fn cursor_offset(&self) -> usize
pub fn cursor_logical_pos(&self) -> Point
pub fn cursor_visual_pos(&self) -> Point
pub fn cursor_move_to_offset(&mut self, offset: usize)
pub fn cursor_move_to_logical(&mut self, pos: Point)
pub fn cursor_move_to_visual(&mut self, pos: Point)
pub fn cursor_move_delta(&mut self, granularity: CursorMovement, delta: CoordType)
```

### 关键调用链

```
键盘事件 → input.rs: EditCommand
  → dispatch/editor.rs
    → commands.rs: execute_edit_command()
      → dv.cursor_move_left()    → tb.cursor_move_delta(Grapheme, -1)
      → dv.cursor_move_up()      → tb.cursor_move_to_logical(Point{x:pos.x, y:pos.y-1})
      → dv.cursor_move_right()   → tb.cursor_move_delta(Grapheme, 1)
      → dv.cursor_move_down()    → tb.cursor_move_to_logical(Point{x:pos.x, y:pos.y+1})
      → dv.cursor_move_word_left()  → tb.cursor_move_delta(Word, -1)
      → dv.cursor_move_to_line_start() → tb.cursor_move_delta(Grapheme, -(col))
    → dv.cursor_move_to_line_end() → tb.cursor_move_to_offset(line_end)
    → dv.move_cursor_visual(delta, ctx) → cursor_motion::move_cursor_visual() → 像素级 sticky_x
```

### 核心问题汇总

| 问题 | 位置 | 影响 |
|------|------|------|
| `usize` 作为字节偏移裸传 | 全链路 | 编译器无法区分 byte offset / grapheme index / visual column |
| `Point` 同时表示 logical 和 visual 坐标 | 全链路 | `logical_pos > visual_pos` 这类非法比较可通过编译 |
| `CoordType = isize` 同时表示列宽和像素宽 | `GraphemeAdvance` trait | 终端列宽 (1-2) 和 GUI 像素宽 (f32→isize) 混用 |
| `Cursor.column` 冗余 | `Cursor` struct | 与 `visual_pos.x` 重复，同步负担 |
| `cursor_move_up/down` 用 logical 而非 visual | `document_view/mod.rs:411-424` | 在折行场景下行为不正确（注释说 "visual line" 但实际用 logical） |
| `sticky_x: f32` 与 `CoordType` 混用 | `cursor_motion.rs` / `cursor_nav.rs` | GUI 像素值 (f32) 与终端列值 (isize) 在同一路径中混用 |

---

## 提案变更

### 第一阶段：引入强类型（core 类型定义层）

#### [NEW] `crates/core/src/types/indices.rs`

引入强类型的 Index 和 Point 定义，从根本上隔离三种坐标空间：

```rust
use std::ops::{Add, Sub};

/// 字节偏移量 — 在 gap buffer 中的绝对位置。
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteIndex(pub usize);

impl ByteIndex {
    pub const ZERO: Self = ByteIndex(0);
    pub const MAX: Self = ByteIndex(usize::MAX);

    pub fn to_usize(self) -> usize { self.0 }
}

// --- 易用性：算术运算符 ---
// 允许 cursor.offset() + 1 而不是 ByteIndex(cursor.offset().to_usize() + 1)

impl Add<usize> for ByteIndex {
    type Output = ByteIndex;
    fn add(self, rhs: usize) -> ByteIndex { ByteIndex(self.0 + rhs) }
}

impl Sub<usize> for ByteIndex {
    type Output = ByteIndex;
    fn sub(self, rhs: usize) -> ByteIndex { ByteIndex(self.0.saturating_sub(rhs)) }
}

impl Sub<ByteIndex> for ByteIndex {
    type Output = usize;
    fn sub(self, rhs: ByteIndex) -> usize { self.0.saturating_sub(rhs.0) }
}

/// 逻辑坐标 — (grapheme_index, logical_line)。
/// 不受折行影响。
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LogicalPoint {
    pub grapheme: usize,  // 从行首计数的 grapheme cluster 索引
    pub line: usize,      // 逻辑行号
}

impl LogicalPoint {
    pub const ZERO: Self = LogicalPoint { grapheme: 0, line: 0 };
    pub const MAX: Self = LogicalPoint { grapheme: usize::MAX, line: usize::MAX };
}

/// 视觉坐标 — (visual_column, visual_row)。
/// 受折行、tab 宽度、CJK 宽字符影响。
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VisualPoint {
    pub column: isize,    // 视觉列宽（终端列或像素，取决于 GraphemeAdvance 实现）
    pub row: usize,       // 折行后的视觉行号
}

impl VisualPoint {
    pub const ZERO: Self = VisualPoint { column: 0, row: 0 };
    pub const MAX: Self = VisualPoint { column: isize::MAX, row: usize::MAX };
}
```

#### [NEW] `crates/core/src/types/mod.rs`

```rust
pub mod indices;
pub use indices::*;
```

在 `crates/core/src/lib.rs` 中注册 `pub mod types;`。

### 第二阶段：改造 Cursor 与 CursorNav

#### [MODIFY] `crates/core/src/unicode/cursor_nav.rs`

**1. 修改 `Cursor` 结构体（行 12-27）**：

```rust
pub struct Cursor {
    pub offset: ByteIndex,
    pub logical_pos: LogicalPoint,
    pub visual_pos: VisualPoint,
    // 删除 column 字段 — 使用 visual_pos.column 替代
}
```

**2. 修改 `CursorNav` 公开 API（行 219-243）**：

```rust
// Before:
pub fn goto_offset(&mut self, offset: usize) -> Cursor
pub fn goto_logical(&mut self, logical_target: Point) -> Cursor
pub fn goto_visual(&mut self, visual_target: Point) -> Cursor

// After:
pub fn goto_byte(&mut self, offset: ByteIndex) -> Cursor
pub fn goto_logical(&mut self, target: LogicalPoint) -> Cursor
pub fn goto_visual(&mut self, target: VisualPoint) -> Cursor
```

**3. 内部 `measure_forward()` 适配（行 250-378）**：

- 入参 `offset_target` 从 `usize` 改为 `ByteIndex`
- `logical_target` 从 `Point` 改为 `LogicalPoint`
- `visual_target` 从 `Point` 改为 `VisualPoint`
- 内部局部变量全部改用强类型
- `calc_target_x` 适配：logical 用 `LogicalPoint`，visual 用 `VisualPoint`
- 哨兵值从 `Point::MAX` / `usize::MAX` 改为各类型的 `::MAX` 常量
- 删除 `column` 字段的维护，统一使用 `visual_pos.column`

**4. 保留 `column` 访问器（兼容层）**：

```rust
impl Cursor {
    pub fn column(&self) -> isize { self.visual_pos.column }
}
```

### 第三阶段：改造 TextBuffer 公开 API

#### [MODIFY] `crates/core/src/buffer/text_buffer.rs`

```rust
// Before:
pub fn cursor_offset(&self) -> usize
pub fn cursor_logical_pos(&self) -> Point
pub fn cursor_visual_pos(&self) -> Point
pub fn cursor_move_to_offset(&mut self, offset: usize)
pub fn cursor_move_to_logical(&mut self, pos: Point)
pub fn cursor_move_to_visual(&mut self, pos: Point)
pub fn cursor_move_delta(&mut self, granularity: CursorMovement, delta: CoordType)

// After:
pub fn cursor_offset(&self) -> ByteIndex
pub fn cursor_logical_pos(&self) -> LogicalPoint
pub fn cursor_visual_pos(&self) -> VisualPoint
pub fn cursor_move_to_byte(&mut self, offset: ByteIndex)    // 显式表明是字节偏移
pub fn cursor_move_to_logical(&mut self, pos: LogicalPoint)
pub fn cursor_move_to_visual(&mut self, pos: VisualPoint)
pub fn cursor_move_delta(&mut self, granularity: CursorMovement, delta: isize)
```

`CursorMovement` 枚举保持不变（`Grapheme` / `Word`）。

### 第四阶段：改造 buffer/navigation.rs 内部实现

#### [MODIFY] `crates/core/src/buffer/navigation.rs`

- `goto_line_start(cursor: Cursor, target_y: CoordType)` → `target_y: usize`
- `cursor_move_to_offset_internal()` 中 `offset: usize` → `ByteIndex`
- `cursor_move_to_logical_internal()` 中 `pos: Point` → `LogicalPoint`
- `cursor_move_to_visual_internal()` 中 `pos: Point` → `VisualPoint`
- `cursor_move_delta_internal()` 中 `delta: CoordType` → `isize`
- 所有 `Point { x, y }` 字面量改为对应的强类型构造

### 第五阶段：改造 app 层调用方

#### [MODIFY] `crates/app/src/document_view/mod.rs`

需要适配的方法（行号来自当前代码）：

| 方法 | 行号 | 改动 |
|------|------|------|
| `cursor_offset()` | 253 | 返回类型 `usize` → `ByteIndex`，调用方需 `.to_usize()` 解包 |
| `cursor_move_left()` | 329 | `CursorMovement::Grapheme, -1` — delta 类型从 `CoordType` → `isize` |
| `cursor_move_right()` | 335 | 同上 |
| `cursor_move_to_offset()` | 341 | 入参 `usize` → `ByteIndex` |
| `cursor_move_up()` | 411 | `Point { x: pos.x, y: pos.y - 1 }` → `LogicalPoint { grapheme: pos.grapheme, line: pos.line - 1 }`，并加 `// TODO: 后续引入 LineMap 后，应改为基于 VisualPoint 移动，以修复折行时的上下漂移问题` |
| `cursor_move_down()` | 420 | 同上 |
| `cursor_move_to_line_start()` | 359 | delta 表达式适配 |
| `cursor_column()` | 534 | 返回类型不变（仍是 `usize` — 这是字节偏移差），内部适配 `ByteIndex` 减法 |

#### [MODIFY] `crates/app/src/document_view/cursor.rs`

`CursorState.offset: usize` → `ByteIndex`

#### [MODIFY] `crates/app/src/cursor_motion.rs`

- `CursorRenderState.last_cursor_offset: usize` → `ByteIndex`
- `CursorRenderState.click_hint` 中的 `usize` 需评估（第二个是 visual line index，保持 `usize`；第一个是 byte offset → `ByteIndex`），类型改为 `Option<(ByteIndex, usize)>`
- `CursorMoveResult::Moved(usize)` → `Moved(ByteIndex)`
- `find_closest_offset()` 返回值 `usize` → `ByteIndex`
- 各辅助函数 (`move_up_past_visible`, `move_down_past_visible`) 的返回值适配

#### [MODIFY] `crates/app/src/commands.rs`

- `execute_edit_command()` 中构造 `Point` 的地方改为对应强类型

#### [MODIFY] `crates/app/src/app_scroll.rs` 及其他引用处

- 所有传递裸 `usize` 作为 cursor offset 的地方改为构造 `ByteIndex(offset)` 或 `.to_usize()` 解包
- 得益于 `ByteIndex` 的 `Add<usize>` / `Sub<usize>` 实现，`offset + 1`、`offset - 1` 等常见模式可以保持不变

### 第六阶段：改造 Cursor 单元测试

#### [MODIFY] `crates/core/src/unicode/cursor_nav.rs` 测试模块（行 407-567）

所有测试中的：
- `Point { x, y }` → 对应 `LogicalPoint` 或 `VisualPoint`
- `Cursor { offset: n, ... }` → `Cursor { offset: ByteIndex(n), ... }`
- `goto_offset(n)` → `goto_byte(ByteIndex(n))`
- `goto_visual(Point { x, y })` → `goto_visual(VisualPoint { column: x, row: y as usize })`

---

## 不做的事情（明确排除）

1. **不引入 `LineMap` 缓存层**：本期聚焦类型安全，O(N) 遍历的性能优化留给后续排版模块。
2. **不改变 `GraphemeAdvance` 的返回值类型**：`CoordType = isize` 保持不变。虽然 `VisualAdvance` 枚举是更干净的方案（见下方"未来工作"），但涉及面太广（`TerminalAdvance`、`PixelAdvance`、`measure_forward` 累加逻辑、所有 advance 调用方），本期不做。
3. **不修改 `selection.rs` 的公开 API 签名**：`selection_update_offset(usize)` 等保持 `usize` 入参，内部转换为 `ByteIndex`。对外接口的强类型化放到后续 PR。
4. **不移除 `Point` 类型**：`helpers::Point` 仍被 viewport、layout、render 等大量非 cursor 模块使用，仅从 cursor 相关路径中移除。
5. **不修复 `cursor_move_up/down` 的折行 Bug**：本次平滑迁移，保持行为不变。仅在代码中加 `// TODO` 注释标记，留待后续引入 `LineMap` 后修复。

---

## 验证计划

### 编译期验证（类型系统保证）

1. 试图将 `LogicalPoint` 传递给 `goto_visual()` → 编译失败
2. 试图将 `ByteIndex` 与 `usize` 做算术运算而不显式转换 → 编译失败（`+`/`-` 运算符除外，它们返回 `ByteIndex`）
3. 试图比较 `LogicalPoint` 与 `VisualPoint` → 编译失败

### 自动化测试

1. `cargo test -p core` — 所有 cursor_nav 测试通过
2. `cargo test -p app` — 所有 cursor_motion 测试通过
3. `cargo test` — 全量测试通过
4. `./scripts/verify.sh` — 全面验证通过

### 手动验证

1. 编译并运行 `edit+`，打开包含中文（你好世界）、Emoji（🎉🚀✨）以及组合字符（é、😶‍🌫️）的测试文档
2. 键盘方向键上下左右移动光标 — 确认无错位
3. 鼠标点击定位 — 确认光标落在正确位置
4. Option+Left/Right 按词跳转 — 确认中英文混排下的词边界正确
5. Shift+方向键选区 — 确认选区范围正确
6. PageUp/PageDown — 确认翻页后光标位置正确

---

## 未来工作

### GraphemeAdvance 返回值强类型化

当前 `GraphemeAdvance` trait 返回 `CoordType = isize`，终端模式返回列宽 (1-2)，像素模式返回 `f32 as isize`，两种语义混在同一类型中。引入 `VisualAdvance` 枚举可以从类型层面隔离：

```rust
/// 视觉前进量的统一抽象。
/// 可以是终端列宽 (u8) 或像素宽 (f32)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VisualAdvance {
    /// 终端列宽 (0, 1, 2)
    Columns(u8),
    /// GUI 像素宽
    Pixels(f32),
}

impl VisualAdvance {
    pub fn to_column(&self) -> isize {
        match self {
            VisualAdvance::Columns(c) => *c as isize,
            VisualAdvance::Pixels(p) => *p as isize,
        }
    }

    pub fn to_pixels(&self) -> f32 {
        match self {
            VisualAdvance::Columns(c) => *c as f32,
            VisualAdvance::Pixels(p) => *p,
        }
    }

    pub fn is_zero(&self) -> bool {
        match self {
            VisualAdvance::Columns(c) => *c == 0,
            VisualAdvance::Pixels(p) => *p <= f32::EPSILON,
        }
    }
}
```

```rust
pub trait GraphemeAdvance: Clone {
    fn advance(&self, cluster: &[u8]) -> VisualAdvance;
    fn tab_advance(&self, current_column: VisualAdvance, tab_size: isize) -> VisualAdvance;
    fn clamp_cluster_width(&self, width: VisualAdvance) -> VisualAdvance;
}
```

涉及模块：`TerminalAdvance`、`PixelAdvance`、`measure_forward` 累加逻辑、所有 advance 调用方。建议在本次重构稳定后作为独立 PR 跟进。
