# edit+ 实施方案

GUI 文本编辑器，目标：极致性能。基于 `microsoft/edit`（终端版）的核心算法，
替换其终端渲染层为 GUI 渲染栈。

## 1. 决策（已敲定）

| 维度 | 选择 |
|---|---|
| 渲染栈 | winit + wgpu + cosmic-text |
| 首发平台 | macOS |
| Text buffer | 抄 edit 的 gap buffer（v2 再考虑 piece tree） |
| 首阶段功能 | 打开/编辑/保存单文件 + 多 buffer/Tab + 搜索/替换（含 ICU 正则） |
| 语言/版本 | Rust 2024 edition，与 edit 对齐 |

## 2. 复用 vs. 自研一览

### 直接 vendor 进来（按文件粒度）

源路径相对于 `../edit/`。

| 来源 | 落地位置 | 备注 |
|---|---|---|
| `crates/stdext/**` | `crates/stdext/` | 全量。arena/BString/BVec/glob/utf8/simd memset 是基础设施。 |
| `crates/lsh/**` | `crates/lsh/` | 全量。语法高亮 DSL+VM+definitions（首阶段不集成，但留着） |
| `crates/edit/src/buffer/**` | `crates/core/src/buffer/` | gap_buffer + navigation + mod.rs |
| `crates/edit/src/document.rs` | `crates/core/src/document.rs` | trait 抽象，整个 buffer/measurement 的根 |
| `crates/edit/src/unicode/**` | `crates/core/src/unicode/` | measurement + tables，**需要参数化**（见 §4） |
| `crates/edit/src/simd/**` | `crates/core/src/simd/` | memchr2 / lines_fwd / lines_bwd |
| `crates/edit/src/icu.rs` | `crates/core/src/icu.rs` | 动态加载 ICU；macOS 用 `libicucore.dylib` |
| `crates/edit/src/fuzzy.rs` | `crates/core/src/fuzzy.rs` | 命令面板/文件搜索 |
| `crates/edit/src/oklab.rs` | `crates/core/src/oklab.rs` | 颜色空间，主题用 |
| `crates/edit/src/{base64,hash,json,path,helpers}.rs` | `crates/core/src/` | 通用 |
| `crates/edit/src/lsh/{cache,highlighter}.rs` | `crates/core/src/lsh/` | 语法高亮绑定层（v2 阶段才接入） |

### 不抄（终端强耦合，GUI 自己写）

| 文件 | 原因 |
|---|---|
| `tui.rs` (4058 行) | 终端 immediate-mode UI |
| `framebuffer.rs` / `cell.rs` / `vt.rs` | cell grid + VT escape |
| `input.rs` | 终端按键/escape sequence 解析 |
| `clipboard.rs` | 终端 OSC 52 |
| `sys/{unix,windows}.rs` | 终端 raw mode |
| `bin/edit/**` | 终端 UI 装配 |

### 核心改造点：unicode/measurement.rs 的像素化（§4 详细设计）

终端版 `MeasurementConfig` 用"列宽"（1 或 2）。GUI 必须换成像素 advance。
**架构保留，单位替换**——这是 plans.md 里设计章节最核心的事。

## 3. 工程结构

```
edit+/
├── Cargo.toml                  # workspace
├── plans.md                    # 本文件
├── CLAUDE.md / AGENTS.md
└── crates/
    ├── stdext/                 # vendored，无修改
    ├── lsh/                    # vendored（含 definitions/）
    ├── core/                   # 从 edit/src 抽出的"算法层"
    │   ├── src/
    │   │   ├── lib.rs
    │   │   ├── document.rs
    │   │   ├── file.rs         # 零拷贝文件加载（自研）
    │   │   ├── buffer/
    │   │   ├── unicode/        # 改造：参数化 advance
    │   │   ├── simd/
    │   │   ├── icu.rs          # sys mod 内联；deferred 部分见 icu.rs.deferred
    │   │   ├── fuzzy.rs
    │   │   ├── oklab.rs
    │   │   └── ...             # base64 / hash / json / path / helpers / cell
    │   └── Cargo.toml
    ├── shaping/                # 新增：cosmic-text 封装
    │   └── src/lib.rs
    ├── render/                 # 新增：wgpu glyph renderer + atlas
    │   └── src/lib.rs
    ├── ui/                     # 占位（阶段 9+）
    │   └── src/lib.rs
    └── app/                    # 新增：bin + lib（winit 事件循环、装配）
        └── src/
            ├── main.rs         # CLI 入口
            ├── lib.rs          # 公开 App / DocumentView / GpuError
            ├── app.rs          # winit ApplicationHandler
            ├── cli.rs          # --headless / file arg
            ├── gpu.rs          # create_gpu_context + headless_init
            ├── viewport.rs     # 滚动跟踪
            └── document_view.rs # GapBuffer + 行索引 + 视口
```

依赖方向（单向，禁止反向）：

```
app  →  ui  →  render  →  shaping
                ↓             ↓
                core  ───────┘
                  ↓
        stdext, lsh
```

## 4. 关键接口设计

### 4.1 Document（直接复用 edit）

```rust
pub trait ReadableDocument {
    fn read_forward(&self, off: usize) -> &[u8];
    fn read_backward(&self, off: usize) -> &[u8];
}
pub trait WriteableDocument: ReadableDocument {
    fn replace(&mut self, range: Range<usize>, replacement: &[u8]);
}
```

`GapBuffer` 实现这两个 trait，所有上层模块只依赖 trait。

### 4.2 Measurement 像素化（核心改造）

终端版 `MeasurementConfig` 内部有一个常量 `wcwidth(grapheme)→{1,2}`。
改造方案：把"宽度"抽象成 trait，注入 shaping 给的像素 advance。

```rust
// crates/core/src/unicode/measurement.rs (改造后)
pub trait GraphemeAdvance {
    /// 给定 grapheme cluster 的 UTF-8 字节，返回视觉宽度。
    /// 单位由实现决定：终端用列(1/2)，GUI 用像素(f32)。
    /// 返回 i32 是为了和原来 CoordType 兼容；GUI 实现把像素*放大系数后取整。
    fn advance(&mut self, cluster: &[u8]) -> CoordType;

    /// tab 在当前列的展开宽度（GUI 用于 stop-tabs）
    fn tab_advance(&mut self, current_column: CoordType) -> CoordType;
}

pub struct MeasurementConfig<'doc, A: GraphemeAdvance> {
    cursor: Cursor,
    word_wrap_limit: CoordType,    // 像素或列，与 advance 同单位
    advance: A,
    buffer: &'doc dyn ReadableDocument,
}
```

两个内置实现：
- `TerminalAdvance`（保留原行为，返回 1/2）—— 测试基线
- `PixelAdvance`（新写）—— 委托给 shaping crate，按 grapheme 查 cosmic-text 的 cached glyph advance

`Cursor` 三坐标语义保留（offset / logical_pos / visual_pos）。`visual_pos.x` 在 GUI 里不再是"列"，而是"像素 / 子像素整数"。所有 buffer/navigation 代码不需要改一行——它们只读 cursor 字段。

**取舍记录**：用 `i32` (CoordType) 不用 `f32`：
1. 子像素抖动只在最终绘制 stage 处理；逻辑层全整数避免浮点累计误差。
2. GUI 用 26.6 定点（× 64）或更粗的 1/16 像素都够。

### 4.3 Shaping（新模块）

```rust
// crates/shaping/src/lib.rs
pub struct ShapeContext { /* cosmic-text FontSystem + SwashCache */ }

pub struct ShapedRun {
    pub glyphs: Vec<ShapedGlyph>,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

pub struct ShapedGlyph {
    pub glyph_id: u32,
    pub font_id: cosmic_text::fontdb::ID,
    pub x_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub cluster: u32,    // 对应原文本字节偏移
}

impl ShapeContext {
    pub fn shape_line(&mut self, text: &str, font_size: f32, attrs: Attrs) -> ShapedRun;

    /// 给 measurement 用：grapheme→advance（带缓存）
    pub fn grapheme_advance(&mut self, cluster: &str, ctx: &CellMetrics) -> i32;
}
```

shaping 用 LRU 缓存 grapheme→advance（key: (cluster_bytes, font_size, attrs_hash)）。
grapheme 数量有限，缓存命中率高。

### 4.4 Render（新模块）

```rust
// crates/render/src/lib.rs
pub struct GlyphRenderer { /* wgpu device/queue, glyph atlas, pipeline */ }

impl GlyphRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self;

    /// 提交一组 glyph，append 到 instance buffer
    pub fn queue_run(&mut self, run: &ShapedRun, origin: Vec2, color: u32);

    /// 提交矩形（光标、选区、行高亮）
    pub fn queue_rect(&mut self, rect: Rect, color: u32);

    /// 一帧 flush
    pub fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView);
}
```

glyph atlas: SDF 还是普通灰度位图？**起步用 cosmic-text + swash 的灰度 alpha mask** + dynamic atlas。SDF 是后期优化项。

### 4.5 UI（新模块）

视图层。先不要 immediate-mode 框架，自己写一个最小版：
- `Editor`：单 buffer 视图，处理输入、滚动、绘制
- `TabBar`：多 buffer 切换
- `StatusBar`：底栏
- `Dialog`：搜索/替换面板（floating）
- `Theme`：颜色/字体配置

事件流：`winit::Event` → `App::handle_event` → 分发到聚焦的 widget。

## 5. 阶段切分

每个阶段独立可交付、能编译能跑。每阶段包含四块验收：
1. **自动化测试**：具体测试名、bench 阈值
2. **手动验收**：可粘贴的步骤 + 预期结果
3. **性能门槛**：硬数字，不达标 = 不通过
4. **边界 case**：必须覆盖的场景清单

测试样本统一放在 `assets/samples/`，定义见 §9。手动操作脚本统一在 `docs/manual_test_protocol.md`，§10。

平台基线：macOS Apple Silicon（M1/M2 及以上），release 构建（`--profile release`），开 LTO + opt-level=s。

---

### 阶段 0：工程骨架（0.5 天）

**目的**：建好 workspace，能编译，vendor 的 stdext/lsh 跑通自带测试。

**自动化**
- `cargo build --workspace` 通过
- `cargo test -p stdext` 全绿（保留 edit 仓库 stdext 全部 inline test）
- `cargo test -p lsh` 全绿
- `cargo clippy --workspace -- -D warnings` 通过
- `cargo fmt --check` 通过（rustfmt.toml 抄 edit）

**手动**
1. `cargo tree -p app` —— 依赖图无环
2. `git ls-files crates/stdext crates/lsh | wc -l` —— 文件数与上游一致

**性能门槛**
- 全量 `cargo build --release` 冷构建 < 90 s（粗指标，监控倒退）

**边界 case**
- 不引入新代码；只验证 vendor 完整性

**交付**：workspace 建立，stdext/lsh 全绿。

---

### 阶段 1：core crate 抽取（1–2 天）

**目的**：把 edit 的算法层独立成 `core` crate，零 GUI 依赖。

**自动化**（必保留 case，名称带原模块前缀）
- `core::document::tests::*`（`String`/`PathBuf` 实现 trait 的 read_forward/backward）
- `core::buffer::gap_buffer::tests::*`（edit 原 case 全抄）
- `core::buffer::navigation::tests::*`
- `core::unicode::measurement::tests::*`（保持 TerminalAdvance 行为）
- `core::simd::memchr2::tests::*`、`lines_fwd::tests::*`、`lines_bwd::tests::*`
- `core::fuzzy::tests::*`
- `core::icu::tests::load_basic`（macOS 加载 libicucore，符号不验证）
- `core::path::tests::*`、`core::hash::tests::*`、`core::base64::tests::*`、`core::json::tests::*`
- `cargo clippy -p core -- -D warnings` 通过

**手动**
1. `rg "use crate::(framebuffer|cell|vt|tui|input|sys::)" crates/core/src/` —— 输出必须为空
2. `cargo doc -p core --no-deps` 无 warning；导出符号至少含 `Document/ReadableDocument/WriteableDocument/GapBuffer/MeasurementConfig/Cursor/Point/CoordType`

**性能门槛**
- 抽取后保留原 measurement bench（`bench_measurement_walk_*`），与 edit 上游 ±5% 内

**边界 case**
- 不引入新功能；只确认 vendor 完整、import 正确
- buffer/mod.rs 涉及 framebuffer 渲染的方法：剥离到独立模块（暂时 `#[cfg(feature = "terminal-render")]`）或直接删除并记入抄码 checklist 备注

**交付**：core crate 编译、测试通过；公开 API 不再依赖任何终端类型。

---

### 阶段 2：measurement 像素化改造（1–2 天）

**目的**：引入 `GraphemeAdvance` trait，参数化宽度单位；行为零回归 + 像素实现就位。

**自动化**
- 阶段 1 全部 measurement 测试改用 `TerminalAdvance`，结果**逐字段**与原版对齐（offset / logical_pos / visual_pos / column / wrap_opp）
- 新增：
  - `core::unicode::measurement::tests::pixel_advance_basic` —— ASCII 单字符 advance
  - `core::unicode::measurement::tests::pixel_word_wrap_at_pixel_limit` —— word_wrap_limit 单位为像素
  - `core::unicode::measurement::tests::pixel_tab_stop_alignment` —— tab 对齐 N×em
  - `core::unicode::measurement::tests::pixel_advance_zero_width` —— ZWJ/变体选择器 advance=0
  - `core::unicode::measurement::tests::pixel_advance_extreme_long_line` —— 1 MiB 单行 word_wrap 不死循环（5 s 超时）
- 新增 bench `bench_measurement_walk_pixel_1mb`，与 `bench_measurement_walk_terminal_1mb` 同语料对比

**性能门槛**
- pixel bench / terminal bench < 1.5×（trait 化的额外开销不应超过 50%）
- `cargo expand -p core unicode::measurement | grep -c "dyn GraphemeAdvance"` = 0（要求静态分发）

**手动**
1. 阶段 1 测试集合先跑 TerminalAdvance —— 0 失败
2. `git diff crates/core/src/unicode/measurement.rs` 检查改动只在 `MeasurementConfig` 泛型化与 trait 注入；navigation 代码零修改
3. 提交 commit 必须能编译

**边界 case**
- grapheme = 单 ASCII
- grapheme = 多字节 CJK
- grapheme = ZWJ 序列（family、肤色 modifier）
- grapheme = 控制字符（\t / \r / \n）
- 极长行 1 MiB 单行
- word_wrap_limit = 0（关闭）
- word_wrap_limit < 单 grapheme advance（强制硬换）
- 子像素：advance 用 26.6 定点，累加不溢出 i32（按 100k 行 × 平均 80 advance 计上限）

**交付**：行为零回归 + PixelAdvance 占位可用；trait 静态分发零开销。

---

### 阶段 3：winit + wgpu 空窗口（1 天）

**目的**：app crate 起 winit + wgpu，跑出空窗口，跑通生命周期。

**自动化**
- `cargo build -p app --release` 通过
- `crates/app/tests/smoke.rs::test_app_init_and_shutdown` —— 环境变量 `EDIT_PLUS_HEADLESS=1` 时跳过 window，仅创建 wgpu instance/adapter/device，5 s 内退出，断言 0 panic
- `crates/app/tests/smoke.rs::test_resize_no_panic` —— headless 下 fake 100 次 resize event

**性能门槛**
- 启动到首帧（即首次 `present`）：cold < 300 ms，warm < 150 ms
- 空窗口 idle CPU：Activity Monitor 抽 30 s 平均 < 0.5%

**手动**
1. `cargo run -p app` —— 出现窗口；标题 "edit+"
2. resize：100×100 → 全屏 → 还原；不卡帧、无残影
3. 主屏 → 外接显示器拖动；DPI 切换不崩
4. Cmd+Q —— 进程 exit code 0
5. 拔/插外接电源（macOS 切换 GPU）—— 不崩

**边界 case**
- 极小窗口（winit 自动 clamp）
- DPI 1× / 2× / 3×（system preference 切换）
- 后台/前台切换
- 所有显示器断开时（仅 headless adapter）

**交付**：开窗、关窗、resize、DPI 全部稳定。

---

### 阶段 4：cosmic-text 接入 + 静态文本渲染（2–3 天）

**目的**：shaping + render crate 上线，渲染一行硬编码文本，多 script 正确。

**自动化**
- shaping crate：
  - `shape_ascii_basic` —— "Hello" → 5 glyph，advance 累加 = ShapedRun.width
  - `shape_cjk_mixed` —— "Hello 世界" cluster 索引正确
  - `shape_emoji_zwj` —— "👨‍👩‍👧" 单 cluster
  - `shape_arabic_rtl` —— 至少不崩，cluster 顺序记录（RTL 完整支持留 backlog）
  - `grapheme_advance_cache_hit` —— 同 grapheme 第二次走缓存（命中率 ≥ 99%）
  - `bench_shape_ascii_1k_chars < 2 ms  (cosmic-text inherent overhead)
  - `bench_grapheme_advance_lookup` < 50 ns（缓存命中）
- render crate：
  - `glyph_atlas_lru_eviction`
  - `atlas_overflow_creates_new_page` —— 单页满后新开页
  - `atlas_subpixel_phases` —— 8 个 subpixel phase 都能进 cache
- app `tests/render_smoke.rs::render_hello_to_png`
  - headless wgpu 渲染 "Hello, edit+" 到 PNG
  - 与 `tests/golden/hello_edit_plus.png` 做 SSIM ≥ 0.99

**性能门槛**
- 单帧 1 行文本渲染 < 2 ms
- atlas 上传单帧字节 < 256 KB（典型场景）

**手动**
1. `cargo run -p app` —— 看到 "Hello, edit+ — 世界 👨‍👩‍👧"
2. 切换主题色（hard-code 切换）—— 文字无 alpha 黑边、无锯齿
3. macOS 缩放 1×/2×/3× 下文字像素清晰
4. 字体大小 8 / 14 / 72 都正常

**边界 case**
- 字体 fallback：CJK + emoji + Latin 混排都有字形
- 缺字情况（指定一个不存在的家族）—— 走系统默认而非 □
- 子像素抖动：8 个 phase 各取一帧 PNG，肉眼对比无断裂
- 软连字（ligature）：基础 Latin "ffi" 至少不渲染错位
- emoji 变体选择器（U+FE0F）正确生效

**交付**：窗口显示多 script 文字；golden image 通过；shaping/render 性能达标。

---

### 阶段 5：只读显示一个文件（1–2 天）

**目的**：CLI 打开文件，渲染可见行，鼠标滚动顺滑。

**自动化**
- core：
  - `core::buffer::tests::open_file_via_path`
  - `core::buffer::tests::detect_line_endings`（CRLF / LF / CR / 混排）
  - `core::buffer::tests::reject_binary_file`（前 8 KB 含 `\0` → 报错）
  - `core::buffer::tests::handle_bom`
- app viewport：
  - `viewport_clamp_top` / `viewport_clamp_bottom` / `viewport_resize`
  - `viewport_only_shapes_visible_lines` —— 100k 行文件，shape 调用 ≤ 视口行 × 2
- bench：
  - `bench_open_50mb_ascii` —— 加载到 GapBuffer 完成
  - `bench_open_50mb_cjk`
  - `bench_open_200mb_ascii`（mmap 路径）
  - `bench_scroll_60s_60fps` —— 模拟滚动，统计 frame time histogram

**性能门槛**
| 指标 | 阈值 |
|---|---|
| 打开 50 MB ASCII | P95 < 80 ms 到首屏 |
| 打开 50 MB CJK | P95 < 200 ms 到首屏 |
| 打开 200 MB ASCII（mmap） | P95 < 400 ms |
| 60 s 持续滚动 | 丢帧 < 1% |
| 50 MB 文件 RSS | < 150 MB |

**手动**（详见 docs/manual_test_protocol.md §5）
1. `cargo run --release -p app -- assets/samples/medium_ascii_5mb.txt` —— 即时首屏
2. `cargo run --release -p app -- assets/samples/large_cjk_50mb.txt` —— 滚轮顺滑到底
3. `cargo run --release -p app -- assets/samples/long_line_1mb.txt` —— 单行不卡，可水平滚或 word wrap
4. `cargo run --release -p app -- assets/samples/illegal_utf8.bin` —— lossy 渲染不崩
5. `cargo run --release -p app -- /dev/null` —— 空窗口
6. `cargo run --release -p app -- nonexistent.txt` —— 友好错误，非 panic
7. 拖动滚动条直接跳到末尾，再跳回头部

**边界 case**
- CRLF / LF / CR-only 混排
- 末尾无换行 / 仅有换行
- 单行 1 MB（高亮可拒绝，渲染不卡）
- 含 BOM（UTF-8 BOM 保留）
- 非法 UTF-8 序列（lossy）
- NFC / NFD 组合字符
- ZWJ 与变体选择器
- 空文件（0 字节）、1 字节
- 文件被外部进程修改（先不处理，但绝不崩）

**交付**：浏览大文件可用；性能门槛全部达标。

---

### 阶段 6：键盘输入 + 编辑（2–3 天）

**目的**：能编辑文本（不保存）；输入延迟达标。

**自动化**
- core 原 buffer replace 测试照抄
- app input：
  - `key_to_command_mapping` —— 全部 macOS 快捷键到 EditCommand 映射
  - `cursor_move_left_at_line_start` —— 跳到上一行末
  - `cursor_move_word_unicode_boundary` —— Option+Left/Right 按 ICU 词边界
  - `backspace_grapheme_cluster` —— ZWJ emoji 一次删完
  - `enter_inserts_native_eol` —— CRLF/LF 文件分别插入对应换行符
  - `delete_at_eof_no_op`
- bench `bench_typing_throughput` —— 1 s 持续 insert ≥ 10 000 次

**性能门槛**
- 输入延迟（按键 → swapchain present）：< 8 ms
- 持续打字 60 s 无掉帧
- 单字符插入引起的 measurement 重算：< 1 ms

**手动**
1. 空文件，敲 "Hello, 世界 🌏"，光标位置正确
2. Backspace 删 🌏 —— 一次删整个 emoji
3. ←→↑↓ + Cmd+方向 + Option+方向 全部按 macOS 习惯
4. Enter 在 CRLF 文件里插入 CRLF；在 LF 文件里插入 LF
5. 长按某键（macOS 字符面板）—— 不触发面板，连续插入字符
6. 选中后输入 —— 替换选区（与阶段 7 衔接，本阶段先验证基础替换）
7. 按 60 字/秒打字 30 s —— 帧率不抖

**边界 case**
- ZWJ emoji 中间放光标（grapheme 边界保护）
- NFD 组合字符中间删除（删整 cluster）
- BOM 后第一个字符（不能误删 BOM）
- 输入 `\0`（替换为 U+FFFD）
- 极长行末 Enter（性能不退化）
- 滚轮 + 键盘并发
- 数字小键盘 NumLock 状态

**交付**：编辑流畅；输入延迟达标；grapheme 边界正确。

---

### 阶段 7：选择 + 剪贴板 + 撤销/重做（2 天）

**目的**：完整选择 + macOS 剪贴板 + undo/redo。

**自动化**
- selection：
  - `mouse_drag_creates_range`
  - `shift_arrow_extends_selection`
  - `shift_click_extends_to_point`
  - `triple_click_selects_line`
  - `double_click_selects_word_unicode`（按 byte-class 分词；CJK 整段视作一词；真正 ICU 词边界推迟到阶段 11）
- buffer history：
  - 若 edit 上游已有 undo：照抄 case
  - 否则新增（在 plans 里列名）：`history_undo_single_insert`、`history_undo_replace`、`history_redo_after_branch_loses_redo_stack`、`history_coalesce_continuous_typing`、`history_limit_memory_cap`
- clipboard：
  - `clipboard_roundtrip_utf8`
  - `clipboard_eol_normalization_on_paste`（外部 CRLF → 内部 LF 文件）
  - `clipboard_strip_bom_on_paste`
- bench `bench_select_1mb_redraw` < 16 ms

**性能门槛**
- copy 1 MB 文本 < 50 ms
- undo/redo 50 步连续操作 < 100 ms 总耗时

**手动**
1. 鼠标拖选；状态栏显示选中字符数 + 字节数
2. Shift+方向键扩选；Cmd+A 全选
3. 双击选词（含 CJK）；三击选行
4. Cmd+C → Safari 粘贴；Safari 复制 → Cmd+V 进编辑器
5. Cmd+Z 撤销 → Shift+Cmd+Z 重做（连续 50 次不丢历史）
6. 选中 → 直接打字 → 替换
7. 外部带 RTF 的剪贴板 —— 只取 plain text

**边界 case**
- 选区跨多行
- 选区跨非法 UTF-8 区域（lossy）
- 剪贴板含 BOM（保留 / 剥除策略明示并测试）
- macOS NSPasteboard 多类型
- undo 越过 load 点（无操作或提示）
- 连续打字合并为单 undo 步

**交付**：选择/剪贴板/撤销重做完整；跨进程剪贴板正确。

---

### 阶段 8：文件 IO 完整闭环（1 天）

**目的**：保存、dirty 标记、关闭确认。

**自动化**
- io：
  - `save_preserves_lf` / `save_preserves_crlf` / `save_preserves_bom`
  - `save_atomic_temp_then_rename`（写中途 SIGKILL 后原文件完整）
  - `save_keeps_file_mode` / `save_keeps_xattr`
  - `dirty_flag_lifecycle`（编辑→脏，保存→洁，外部无变化）
  - `close_dirty_prompts_dialog`（mock 对话框三按钮）
- 错误注入：
  - `save_disk_full_returns_error`
  - `save_readonly_target_returns_error`

**性能门槛**
- 保存 50 MB 文件 < 200 ms
- 原子性：写过程中 kill -9 后磁盘上原文件 SHA-256 不变

**手动**
1. 修改 → Cmd+S → mtime 更新；`shasum` 验证内容
2. CRLF 文件 Cmd+S 后 `file` 仍报 CRLF
3. 关闭未保存窗口 → 三按钮对话框（Save / Don't Save / Cancel）
4. 标题栏 dirty 圆点（macOS NSWindow.documentEdited）
5. Cmd+Shift+S 另存为
6. 外部 `echo > file` 修改后 —— 不崩；提示重载（可选 backlog）

**边界 case**
- 路径含空格、中文、emoji
- 符号链接（写入指向目标）
- 只读文件 → 引导 Save As
- HFS+ case-insensitive
- 跨 volume 保存
- 网络盘（SMB/AFP）

**交付**：单文件编辑闭环；保存原子；EOL/BOM/dirty 全部正确。

---

### 阶段 9：多 buffer + Tab UI（2–3 天）

**目的**：多文档管理 + Tab UI + macOS 文件对话框。

**自动化**
- documents：
  - `open_duplicate_path_focuses_existing`
  - `close_active_switches_to_neighbor`
  - `close_dirty_prompts_then_keeps_or_discards`
  - `tab_reorder_drag`
  - `recent_closed_restore`（Cmd+Shift+T）
- tabbar：
  - `layout_overflow_scroll`
  - `layout_truncate_long_filename`

**性能门槛**
- 同时打开 100 个 buffer：内存增量 < 50 MB（不含文件正文）
- Tab 切换重绘 < 16 ms
- 100 个 tab 渲染单帧 < 4 ms

**手动**
1. Cmd+O → NSOpenPanel → 选文件 → 新 tab
2. Cmd+T 新建空 buffer；Cmd+W 关闭；Cmd+Shift+T 恢复
3. Cmd+1..9 跳到第 N 个
4. 拖拽 tab 重排
5. 拖入多个文件到窗口 → 全部打开
6. 关含 dirty 的 tab —— 与阶段 8 对话框一致

**边界 case**
- 100 tab 性能不退化
- 同文件外部修改 —— 多 tab 显示不同内容（不同步，不崩）
- HFS case-insensitive 同 path 视为同 buffer
- 极长文件名（中间省略）

**交付**：多 buffer 工作流完整；macOS 原生对话框集成。

---

### 阶段 10：搜索（无正则，SIMD 直找）（1–2 天）

**目的**：Cmd+F 简单搜索 + 全部高亮 + F3 跳转。

**自动化**
- core search：
  - `find_ascii`
  - `find_cjk_byte_aligned`（不切 UTF-8）
  - `find_case_insensitive_ascii`
  - `find_overlapping_matches`（如 "aa" in "aaaa" → 3 个匹配）
  - `find_empty_pattern_returns_empty`
- bench：
  - `bench_find_50mb_ascii_throughput` ≥ 5 GB/s（SIMD）
  - `bench_find_50mb_cjk_throughput` ≥ 1 GB/s

**性能门槛**
- 50 MB 文件全文搜索 < 50 ms 给出全部匹配
- 边输边搜：每字符触发 < 16 ms

**手动**
1. Cmd+F 弹搜索面板
2. 输入 "world"，全文高亮，当前匹配滚到视口
3. F3 / Shift+F3 上下跳；末尾循环回首
4. Esc 关面板；光标停在最后高亮
5. 输入"世界"
6. 大小写敏感开关 → 命中数实时变化
7. 极多匹配（1M+）—— UI 显示前 N 条 + 总数

**边界 case**
- 搜索串跨 grapheme（不跨 UTF-8 byte 边界）
- 末尾匹配
- 空文档搜索
- 搜索串非法 UTF-8（提示，拒绝）
- 极多匹配（1M+），高亮渲染不卡
- 搜索同时编辑 —— 失效后重搜

**交付**：搜索可用；性能达标。

---

### 阶段 11：替换 + ICU 正则（2–3 天）

**目的**：接入 ICU，正则搜索/替换完整。

**自动化**
- core icu：
  - `load_libicucore_macos`（必须加载成功）
  - `regex_basic_compile_and_match`
  - `regex_unicode_categories`（`\p{L}`、`\p{Han}`）
  - `regex_replace_capture_groups`（`$0`/`$1`）
  - `regex_invalid_pattern_returns_error`（不崩）
- app replace：
  - `replace_one`
  - `replace_all`
  - `undo_after_replace_all_single_step`（一键回滚整批）
- bench：
  - `bench_regex_50mb_simple` < 500 ms
  - `bench_undo_10k_replace` < 100 ms

**性能门槛**
- 50 MB 简单 pattern 全文搜索 < 500 ms
- 1 万次替换 undo < 100 ms

**手动**
1. Cmd+H 替换面板
2. 切正则：`(\d+)` → 替换 `[$1]`
3. Replace / Replace All；Replace All → Cmd+Z 单步回滚
4. 大小写敏感切换
5. 输入非法正则 → 红边 + 错误提示
6. `\p{Han}` 命中所有汉字
7. ICU 加载失败注入（`DYLD_LIBRARY_PATH=` 改成不存在路径）—— 退化到普通搜索 + 顶栏提示

**边界 case**
- 替换串含 `$0`..`$9` 反向引用
- 替换扩张文档（1B → 1KB）
- 替换收缩文档（1KB → 1B）
- 病态正则（`(?:)*`）—— ICU 自带保护，不死锁
- 替换导致光标失效 —— 落在最近合法位置

**交付**：正则搜索/替换完整；ICU 加载稳定；降级路径有效。

---

### 阶段 12：性能基准 + 优化（持续，至少 2 天集中）

**目的**：建立基线 + 识别瓶颈 + 文档化。

**自动化**
- 完整 `cargo bench -p core` → `docs/perf_baseline.json`（JSON 落盘，便于对比）
- 加 GUI 端 bench：
  - `bench/render_60fps.rs` —— 模拟滚动 60 s，输出 frame time histogram
  - `bench/input_latency.rs` —— winit fake event 注入到 swapchain present 的 wall time
- CI 守门：bench 退化 > 10% 失败

**性能门槛（首发硬指标）**
| 指标 | 阈值 |
|---|---|
| 打开 100 MB 文件 | < 200 ms |
| 滚动稳定 60 fps | 丢帧 < 1% |
| 输入到屏幕 | < 8 ms |
| 50 MB 全文搜索 | < 50 ms |
| 50 MB 正则搜索 | < 500 ms |
| RSS（50 MB 文件） | < 150 MB |
| 启动到首帧 | < 300 ms |

**手动**
1. Instruments Time Profiler 跑滚动场景，trace 存档
2. `MTL_HUD_ENABLED=1` 观察 GPU/CPU 利用率
3. 5 种语料 × 60 Hz / 120 Hz 双场景

**边界 case**
- 不达标项必须列入 `docs/perf_notes.md`，含原因 + 复现命令
- 任何回归 ≥ 10% 必须 root cause（CLAUDE.md §7 推翻方案，不打补丁）

**交付**：`docs/perf_baseline.md`（数字表）+ `docs/perf_notes.md`（瓶颈与优化清单）；硬指标达标或有书面妥协。

### 后续阶段（不在首发范围内）

| 阶段 | 内容 |
|---|---|
| 13 | 接入 LSH 高亮（直接 vendor 整套 `.lsh`） |
| 14 | 命令面板（Cmd+P/Cmd+Shift+P，复用 fuzzy） |
| 15 | 设置文件 + 主题 |
| 16 | Windows 移植（DirectWrite + Win32 剪贴板） |
| 17 | Linux 移植（fontconfig + Wayland/X11） |
| 18 | piece tree 替换 gap buffer（如有性能需要） |
| 19 | LSP 客户端 |
| 20 | Mini-map / 多光标 / 折叠 |

## 6. 风险与决策记录

### R1：cosmic-text 在大段 CJK/emoji 文本下的 shaping 成本
- 缓解：grapheme→advance 强缓存；只 shape 可见行；垂直滚动复用上次 shape 结果
- 验证点：阶段 5 用 50 MB 含 CJK 的样本

### R2：measurement 像素化破坏 buffer 的换行假设
- edit 原代码的"列"在 navigation/word-wrap 处都是整数运算
- 用整型像素（× 64 的子像素）代替"列"，全部代码不动
- 验证点：阶段 2 的 TerminalAdvance 测试集 100% 通过

### R3：ICU 在 macOS 上 `libicucore` 的 SONAME/符号
- edit 的 `icu.rs` 已经处理了 macOS（`libicucore.dylib`，无前缀符号）
- 验证点：阶段 11 跑通 ICU 加载并执行一个基础正则

### R4：wgpu 在 macOS 上的合成器 vsync 与 promotion
- 默认 fifo presentation；后续可切 fifo_relaxed/mailbox
- ProMotion 自适应刷新率：观察是否需要主动设置目标帧率

### R5：IME（输入法）
- 首发不做。macOS 中文/日文输入需要监听 `Ime` 事件 + 预编辑串渲染
- 列入阶段 9 之后的 backlog；用占位让 winit 的 IME 事件至少不崩

## 7. 抄码 checklist（阶段 1 用）

逐文件确认。`y` = 直接抄；`y*` = 抄但删 import；`r` = 改造；`n` = 不抄。

### 从 edit 抄入的文件

| 文件 | 处理 | 备注 |
|---|---|---|
| `document.rs` | y | trait 不变 |
| `buffer/mod.rs` | y* | 缩减为 gap_buffer + navigation 的 re-export；TextBuffer 延迟到 stage 6（见 `buffer/text_buffer.rs.deferred`） |
| `buffer/gap_buffer.rs` | y | |
| `buffer/navigation.rs` | y | |
| `unicode/mod.rs` | y | |
| `unicode/tables.rs` | y | |
| `unicode/measurement.rs` | r | §4.2 改造：GraphemeAdvance trait + TerminalAdvance / PixelAdvance |
| `simd/*.rs` | y | |
| `icu.rs` | y* | sys.rs 已并入为内联 mod sys；TextBuffer 依赖部分延迟（见 `icu.rs.deferred`） |
| `fuzzy.rs` | y | |
| `oklab.rs` | y | |
| `helpers.rs` | y | CoordType 等基础类型 |
| `hash.rs` | y | |
| `base64.rs` | y | |
| `json.rs` | y | |
| `path.rs` | y | |
| `cell.rs` | y | SemiRefCell 工具（通用，无终端依赖） |
| `clipboard.rs` | n | macOS 走 arboard |
| `framebuffer.rs` | n | 终端渲染，不抄 |
| `vt.rs` | n | VT escape，不抄 |
| `tui.rs` | n | 终端 UI，不抄 |
| `input.rs` | n | 终端按键，不抄 |
| `sys/*` | n | 已并入 icu.rs |
| `lsh/cache.rs` | defer | 推到阶段 13 |
| `lsh/highlighter.rs` | defer | 推到阶段 13 |
| `lsh/definitions.rs` | defer | 推到阶段 13 |
| `bin/edit/**` | n | 终端 UI 装配 |

### 自研文件（不在 edit 中）

| 文件 | 用途 |
|---|---|
| `core/src/file.rs` | 零拷贝文件加载（allocate_gap + commit_gap） |
| `app/src/cli.rs` | CLI 参数解析 |
| `app/src/gpu.rs` | GPU 初始化（create_gpu_context + headless_init） |
| `app/src/viewport.rs` | 视口滚动跟踪 |
| `app/src/document_view.rs` | 文档视图（GapBuffer + 行索引 + 视口） |

## 8. 命令汇总（待 Cargo workspace 建好后填入 CLAUDE.md）

```sh
cargo build --release                  # 构建
cargo run -p app -- <file>             # 启动
cargo test -p core                     # 测试核心算法层
cargo bench -p core                    # buffer/measurement 基准
cargo clippy --all-targets             # lint
cargo fmt                              # 格式（rustfmt.toml 抄 edit）
```

## 9. 样本语料库 `assets/samples/`

阶段 0 起就要建好。所有阶段共用，不允许各阶段临时找文件。
所有样本由脚本生成 `scripts/gen_samples.sh`，结果可重现（固定 seed），不入 git；提供 SHA-256 清单 `assets/samples/SHA256SUMS`。

| 文件 | 大小 | 内容 | 用途 |
|---|---|---|---|
| `tiny_empty.txt` | 0 | 空文件 | 边界：空文档 |
| `tiny_one_byte.txt` | 1 B | `a` | 边界：仅 1 字节 |
| `tiny_no_eol.txt` | ~50 B | 无末尾换行 | EOL 处理 |
| `small_ascii.txt` | 4 KB | Lorem ipsum | 基础 |
| `small_crlf.txt` | 4 KB | CRLF 行尾 | EOL 保留 |
| `small_cr_only.txt` | 4 KB | 仅 CR（老 Mac） | EOL 兼容 |
| `small_mixed_eol.txt` | 4 KB | LF/CRLF/CR 混排 | 混排回归 |
| `small_bom.txt` | 4 KB | UTF-8 BOM | BOM 保留 |
| `small_cjk.txt` | 16 KB | 中日韩混排 | grapheme/shaping |
| `small_emoji_zwj.txt` | 8 KB | family/skin-tone modifier | grapheme 边界 |
| `small_combining.txt` | 8 KB | NFC + NFD 组合字符 | 归一化无关性 |
| `small_zero_width.txt` | 4 KB | ZWJ + 变体选择器 | advance=0 |
| `small_rtl.txt` | 4 KB | 阿拉伯/希伯来 | RTL 兜底 |
| `small_illegal_utf8.bin` | 4 KB | 故意非法字节 | lossy |
| `medium_ascii_5mb.txt` | 5 MB | 重复 Lorem | 中文件 |
| `large_ascii_50mb.txt` | 50 MB | 重复 Lorem | 大文件性能 |
| `large_cjk_50mb.txt` | 50 MB | 重复 CJK | 大文件 + 复杂 shaping |
| `huge_ascii_200mb.txt` | 200 MB | 重复 Lorem | mmap 路径 |
| `long_line_1mb.txt` | 1 MB | 单行 1 MB | 极长行 |
| `long_line_no_eol.txt` | 1 MB | 单行 1 MB 无 EOL | 极端组合 |
| `binary_with_nulls.bin` | 8 KB | 前 8 KB 含 `\0` | 拒绝 binary |
| `path_with_spaces 中文 🌏.txt` | 4 KB | ASCII 内容 | 路径名兼容 |
| `symlink_to_small.txt` | symlink | 指向 small_ascii | 写入跟随 |
| `readonly.txt` | 4 KB, mode 0444 | 只读 | 保存失败路径 |

生成脚本要求：
- 幂等；重复运行结果字节级一致
- 失败重跑；不依赖网络
- macOS / Linux 通用 bash + perl/python，不依赖 GNU 特定 flag

## 10. 手动测试协议 `docs/manual_test_protocol.md`

阶段 ≥ 3 起，每阶段对应一节（§5/§6/...）。规则：
- plans.md §5 各阶段"手动"块只列**步骤序号 + 一句话标题**；详细步骤、预期结果、截图位置都在 manual_test_protocol.md
- 每条步骤可独立执行，不依赖前一条
- 预期结果用「✅ 必须 / ⚠️ 警告 / ❌ 不允许」三档标注
- 失败时记录到 `docs/manual_test_runs/<date>-<stage>.md`，含 macOS 版本、芯片、显示器配置

骨架（阶段 5 示例，落地阶段 5 完成时填）：

```markdown
# 手动测试协议

## §5 阶段 5：只读显示文件

### M5.1 中型 ASCII 即时首屏
命令：`cargo run --release -p app -- assets/samples/medium_ascii_5mb.txt`
预期：
- ✅ 200 ms 内出现首屏文本
- ✅ 滚轮顺滑滚动到末尾
- ✅ 内存（Activity Monitor）增量 < 50 MB
- ❌ 无任何 panic / 警告日志

### M5.2 大型 CJK 滚动
... （略）
```

跨阶段共享的快捷键表：另起一节 `## 快捷键速查`。
