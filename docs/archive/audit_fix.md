# 审计修复方案

针对 2026-05-28 当前代码状态的审计结论与修复计划。本文件是 plans.md 的补丁，
不替代它。修复完成后逐项删除/勾选。

## 0. 整体方向问题

按 plans.md 阶段切分，**阶段 1（core 抽取）和阶段 4（shaping/render）必须串行交付**。
当前把两个阶段揉在一起做了，结果是：

- core 还没干净（仍依赖终端 stub）
- shaping/render 已经写了 600+ 行，但 core 还没稳定，shaping 的接口随时可能要改
- 单元测试覆盖率不均：core/measurement 100+ 个 case，render 13 个，shaping 部分 case 跳过（无字体）

**修复原则**：先把阶段 1 做干净，再回到阶段 2，最后回到阶段 4。
不删 shaping/render 已写代码，但**冻结其接口**直到 core 通过验收。

阶段 3（winit + wgpu 空窗口）的代码还可用，但 `expect`/`unwrap` 太多，需要错误处理收口。

---

## 1. 严重问题（违反设计原则）

### S1. `core/src/file.rs` 方向错了

**现象**：整文件 → `Vec<u8>` → `String::from_utf8_lossy().into_owned()` → `gap_buffer.replace(0..0, bytes)`。

**后果**：
- 内存峰值 = 文件大小 × 3（raw Vec + String + GapBuffer）
- 50 MB 文件即超 plans.md §5 的 RSS < 150 MB 门槛
- `String::from_utf8_lossy` 把非法 UTF-8 替换成 `U+FFFD`，**改变了原始字节**——保存后会污染源文件

**修复**：删除整个 `crates/core/src/file.rs`。在阶段 5 落地时按 edit `buffer/mod.rs:707` 的 `read_file` 路径重写：

```
read_file(file, encoding):
  1) 读前 N 字节 → 探测 BOM/编码
  2) GapBuffer::clear()
  3) 循环：
       gap = buffer.allocate_gap(text_length, chunk_size, 0)
       n = file.read(gap)
       buffer.commit_gap(n)
  4) 单次扫描已读全文 → 行数/CRLF 比例/缩进
```

零拷贝、按需 commit 物理页。这是 GapBuffer 设计的核心价值。

**验收**：50 MB 文件加载后 RSS 增量 < 60 MB（基本就是文件本身大小）。

---

### S2. `buffer/mod.rs` 抄了 3189 行但没真正抽离终端依赖

**现象**：`crates/core/src/terminal_stubs.rs` 提供假类型：

```rust
pub struct Language;
pub struct Clipboard;
pub struct Framebuffer;
pub struct Highlighter;
pub enum IndexedColor { Green, ... }
pub enum HighlightKind { Other, ... }
```

`buffer/mod.rs` 引用这些 stub，`TextBuffer` 公共 API 暴露假类型；`lsh/cache.rs` 也是空壳（`invalidate_from(_)` 不做事）。

**违反**：plans.md 阶段 1 验收"`rg "use crate::(framebuffer|cell|vt|tui|input|sys::)"` 输出必须为空"。

**修复（推荐方案 B）**：缩减 core 抽取范围。

阶段 1 只抄：
- `document.rs`、`buffer/gap_buffer.rs`、`buffer/navigation.rs`
- `unicode/{mod,measurement,tables}.rs`
- `simd/**`
- `helpers.rs`、`oklab.rs`、`hash.rs`、`base64.rs`、`json.rs`、`path.rs`
- `icu.rs`（动态加载，无终端依赖）
- `fuzzy.rs`

**不抄**（推到阶段 6/7）：
- `buffer/mod.rs` 的 `TextBuffer`（含光标、selection、history、render）—— 这层和 UI 强耦合，到阶段 6 才需要
- `lsh/cache.rs`、`lsh/highlighter.rs` —— 推到阶段 13

**做法**：
1. 把现有 `buffer/mod.rs` 移到 `buffer/text_buffer.rs.deferred`（保留代码但不进 `mod.rs`）
2. 新建一个最小 `buffer/mod.rs`：

```rust
mod gap_buffer;
mod navigation;
pub use gap_buffer::GapBuffer;
pub use navigation::*;  // 视实际导出而定
```

3. 删除 `core/src/terminal_stubs.rs`
4. 删除 `core/src/lsh/`
5. `core/src/lib.rs` 去掉 `pub mod cell;` 改成 `mod cell;`（如果 navigation 不用就直接删）

**验收命令**：
```sh
rg "use crate::(framebuffer|cell::|vt|tui|input|terminal_stubs|HighlightKind|Highlighter|Framebuffer|Clipboard|Language|IndexedColor)" crates/core/src/
```
输出必须为空。

---

### S3. `core/src/file.rs::detect_line_ending` 写了两遍

**现象**：第 71-90 行第一次循环的 `has_lf/has_crlf/has_cr` 完全 dead，第 92-109 行重算一次。这是未删干净的旧实现，编译器警告 `has_lf is never read` 就是它。

**修复**：随 S1 一起删除整个文件。如果 EOL 检测保留，新实现走原始 `&[u8]`，不要先转 `String`（lossy 会改字节）。

---

## 2. 中等问题

### M1. shaping cache 是无界 HashMap，不是 LRU

**现象**：
```rust
pub struct GraphemeAdvanceCache {
    cache: HashMap<String, f32>,  // 无淘汰
    hits: u64,
    misses: u64,
}
```

**问题**：
- key 是 `String`，每次插入堆分配
- 无容量上限，CJK + emoji 组合空间巨大，长期跑会爆内存
- plans.md §4.3 设计是 LRU + 复合 key `(cluster_bytes, font_size, attrs_hash)`

**修复**：
1. 加 `hashlink = "0.10"` 依赖（或 `lru` crate）
2. key 改成结构体：
```rust
#[derive(Hash, PartialEq, Eq)]
struct AdvanceKey {
    cluster: SmolStr,           // 多数 grapheme 短，smol_str 内联
    font_size: u32,             // 26.6 定点
    attrs_hash: u64,
}
```
3. 容量上限：8192 条（够覆盖一篇文档的常见 grapheme）
4. 命中率统计保留

**验收**：bench `bench_grapheme_advance_lookup` < 50 ns 命中；满容后插入触发淘汰，命中率不退化。

---

### M2. shaping `Buffer::new` 每次调用都新建

**现象**：
```rust
pub fn shape(&mut self, text: &str) -> Result<ShapedRun, ShapeError> {
    let mut buffer = Buffer::new(&mut self.font_system, metrics);  // 每行 new
    ...
}
```

**问题**：cosmic-text `Buffer` 内部有 line cache 和 shape 结果缓存，反复 new 等于自废武功。

**修复**：把 `Buffer` 作为 `Shaper` 的字段长期持有，每次 `set_text` 复用：
```rust
pub struct Shaper {
    font_system: FontSystem,
    buffer: Buffer,             // 长期持有
    cache: GraphemeAdvanceCache,
    ...
}
```

注意：`Buffer` 的 `Metrics` 改变时需要 `set_metrics`，不要 new 整个 Buffer。

**验收**：bench `bench_shape_ascii_1k_chars` 比当前实现至少快 2×。

---

### M3. shaping `Default for Shaper` 死代码 + 静默吞错

**现象**：
```rust
impl Default for Shaper {
    fn default() -> Self {
        Self::new().unwrap_or(Self { font_system: FontSystem::new(), ... })
    }
}
```

`new()` 当前永远 `Ok`，`unwrap_or` 是死分支；未来 `new()` 真能失败时这里会静默给出无字体 Shaper。违反 CLAUDE.md "别叠加防御"。

**修复**：直接删 `impl Default`。Shaper 强制走 `new() -> Result<Self, ShapeError>`，调用方明确处理错误。

---

### M4. `GlyphAtlas::get` 是 O(N) LRU

**现象**：
```rust
pub fn get(&mut self, key: &GlyphKey) -> Option<&GlyphSlot> {
    if self.slots.contains_key(key) {
        self.access_order.retain(|k| k != key);  // O(N)
        self.access_order.push(*key);
        ...
}
```

`evict_lru` 用 `Vec::remove(0)` 同样 O(N)。N=8192 时每次查询 8K 比较，热路径完全废掉。

**修复**：换 `hashlink::LinkedHashMap`：
```rust
pub struct GlyphAtlas {
    pages: Vec<AtlasPage>,
    slots: LinkedHashMap<GlyphKey, GlyphSlot>,  // 按插入顺序，O(1) 移到末尾
    ...
}
```

`get` 用 `to_back`（O(1)），`evict_lru` 用 `pop_front`（O(1)）。

**验收**：N=10000 下 1M 次 `get` < 100 ms。

---

### M5. `GlyphAtlas::insert` 缺溢出页缓存

**现象**：当 glyph 比一整页还大时，`new_page.allocate` 失败，但 `new_page` 已经创建了——下次再来同样大小的 glyph 又重新分配 + 丢弃。

**修复**：在 `GlyphAtlas` 加 `oversized: HashSet<GlyphKey>`，命中后直接返回 None，不要重复尝试。或者 fallback 到独立的 oversized texture（更晚再做）。

阶段 4 暂不投入精力，加 TODO 注释 + 在 `oversized` set 里记 negative result。

---

### M6. `app::resumed` 里到处 `expect`/`unwrap`

**现象**：
```rust
let window = ...expect("failed to create window");
let surface = ...unwrap();
let adapter = ...expect("no GPU adapter");
let (device, queue) = ...expect("failed to create device");
```

**违反**：plans.md 阶段 3 验收"close、resize 都不崩"。沙盒/无 GPU 环境下进程直接 crash 而非给用户提示。

**修复**：`init_window` 改成 `Result<(), AppError>`，错误向上冒到 `main`，由 main 决定是 print + exit(1) 还是弹原生对话框。

```rust
fn init_window(&mut self, event_loop: &ActiveEventLoop) -> Result<(), AppError> {
    let window = Arc::new(event_loop.create_window(attrs)?);
    let surface = instance.create_surface(window.clone())?;
    let adapter = block_on(...).ok_or(AppError::NoAdapter)?;
    let (device, queue) = block_on(adapter.request_device(...))?;
    ...
}
```

---

### M7. `app::headless_init` 与 `app::init_window` 重复

**现象**：两套近乎一样的 `instance/adapter/device` 创建代码，分歧点只在 `compatible_surface`。

**修复**：抽 `fn create_gpu(surface: Option<&wgpu::Surface>) -> Result<(Adapter, Device, Queue), AppError>`，两边共用。

---

### M8. `core/src/sys.rs` 命名误导

**现象**：文件名 `sys.rs` 看上去像通用平台抽象层，但实际只含 ICU 加载（`load_icu`、`get_proc_address`）。

**修复**：随 S2 一起处理。两个选项：
- 改名 `core/src/icu_loader.rs`，`mod icu_loader;` 私有
- 把这些函数直接并入 `core/src/icu.rs`（icu.rs 里本来就引用它们）

后者更内聚，推荐。

---

## 3. 小问题

### s1. 编译警告 3 处必须清

```
warning: value assigned to `has_lf` is never read    (file.rs:67, 86)
warning: unused import: `WriteableDocument`           (gap_buffer.rs:373)
```

CLAUDE.md "每次提交要确保能编译过"。修完应该 `cargo build --workspace 2>&1 | grep -i warning` 输出空。

---

### s2. `file.rs::contains_null` 多余封装

```rust
fn contains_null(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}
```

`bytes.contains(&0)` 已经够清楚。CLAUDE.md "三行相似比早抽象好"，单行不需要包函数。

随 S1 删除。

---

### s3. plans.md §7 抄码 checklist 与现实不符

| 文件 | checklist 标记 | 实际 |
|---|---|---|
| `cell.rs` | `n`（不抄） | 抄了，是 `SemiRefCell`（合法工具） |
| `lsh/cache.rs` | `y`（直接抄） | 是 stub，应该推到阶段 13 |
| `lsh/highlighter.rs` | `y` | 没抄 |
| `lsh/definitions.rs` | `y` | 没抄 |
| 新增 `file.rs` | 不在表里 | 删除（S1） |
| 新增 `terminal_stubs.rs` | 不在表里 | 删除（S2） |
| 新增 `sys.rs` | 不在表里 | 并入 icu.rs（M8） |

**修复**：plans.md §7 的"备注"列把后三行改成"defer to stage 13"；`cell.rs` 改成 `y`。

---

## 4. 修复执行顺序

| # | 操作 | 依赖 | 估时 |
|---|---|---|---|
| 1 | 删 `core/src/file.rs` | — | 5 min |
| 2 | 缩减 buffer/mod.rs：移到 .deferred，新写最小 mod.rs | — | 30 min |
| 3 | 删 `core/src/terminal_stubs.rs`、`core/src/lsh/` | 2 | 5 min |
| 4 | M8 把 `sys.rs` 并入 `icu.rs`，删 `sys.rs` | — | 15 min |
| 5 | 清编译警告（s1） | 1, 2 | 10 min |
| 6 | 跑 `cargo test -p edit-plus-core`，确认 measurement/simd/helpers/oklab/json 等仍全绿 | 1-5 | 5 min |
| 7 | M1 shaping cache LRU + 复合 key | — | 1 h |
| 8 | M2 shaping Buffer 复用 | 7 | 30 min |
| 9 | M3 删 Default | 7 | 5 min |
| 10 | M4 atlas LinkedHashMap | — | 30 min |
| 11 | M5 atlas oversized set + TODO | 10 | 15 min |
| 12 | M6 app 错误处理 | — | 30 min |
| 13 | M7 抽 create_gpu | 12 | 20 min |
| 14 | 更新 plans.md §7 checklist（s3） | 1-13 | 10 min |
| 15 | 全量验收：build / test / clippy -D warnings / fmt | all | 15 min |

合计约 4–5 小时。

---

## 5. 验收清单（修完一遍跑，记录结果）

```sh
# 必须空输出
rg "use crate::(framebuffer|cell::|vt|tui|input|terminal_stubs|HighlightKind|Highlighter|Framebuffer|Clipboard|Language|IndexedColor)" crates/core/src/

# 必须 0 warnings
cargo build --workspace 2>&1 | grep -i warning

# 必须全绿
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check

# 必须能跑
EDIT_PLUS_HEADLESS=1 cargo run -p edit-plus-app
cargo run -p edit-plus-app   # 弹窗，Esc 退出
```

跑完把结果记到本文件的 §6 里，每条勾掉。

---

## 6. 验收记录

待修复后填入。

```
[ ] core 无终端依赖（rg 输出空）
[ ] 编译零警告
[ ] cargo test --workspace 全绿
[ ] clippy -D warnings 通过
[ ] cargo fmt --check 通过
[ ] headless 模式启动 < 300ms
[ ] 窗口模式启动 + Esc 退出 exit 0
[ ] plans.md §7 checklist 已更新
```
