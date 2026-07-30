# 冷启动性能优化方案

> 创建日期：2026-06-01
> 范围：从进程 `main()` 到第一帧像素呈现的完整链路
> 目标：拿到真实数字 → 全面优化（A–E）→ 按实测收益排序落地

---

## 1. 背景与目标

### 1.1 当前观察

- macOS（Apple M5 / Metal）上 `--headless` warm cache 实测约 10ms，cold cache 首次 ~680ms。
- 主进程 `main → resumed → init_window → init_text → load_file → 第一帧 redraw` 全程串行同步，期间窗口不可见。
- `pollster::block_on` 在主线程对 wgpu adapter/device/configure 阻塞。
- `Shaper::new()` 调用 `cosmic_text::FontSystem::new()`，会枚举/解析整个系统字体库（macOS 上常见 100ms+）。
- 第一帧之前没有任何「窗口可见」的中间状态，用户感知 = 链路总和。

### 1.2 目标

| 维度 | 当前估算 | 目标 |
|------|---------|------|
| 进程启动 → 窗口可见（warm） | 串行总和（>200ms 估算） | < 50ms |
| 进程启动 → 第一帧文本（warm） | 串行总和 | < 120ms |
| 进程启动 → 窗口可见（cold） | >700ms | < 200ms |
| 大文件首屏（>10 MB） | 同步阻塞 | 与小文件等同（首屏先 paint） |

> 数字会在 Phase 0 实测后修正成具体阈值。

### 1.3 非目标

- 不优化运行期编辑/滚动延迟（已有独立优化）。
- 不替换 winit / wgpu / cosmic-text 主体方案。
- 不引入额外的 GUI runtime（如 tokio）；保持线程模型简洁。

---

## 2. 冷启动链路全景

按时间顺序，每一步对应文件位置、当前耗时估算、性质：

| # | 阶段 | 代码位置 | warm 估算 | cold 估算 | 类型 | 优化方向 |
|---|------|---------|-----------|-----------|------|---------|
| 1 | CLI 解析、EventLoop 创建 | `main.rs:5,19` | <2ms | <5ms | 必要 | — |
| 2 | `event_loop.run_app` → `resumed` 回调 | `main.rs:21` / `app.rs:1473` | <2ms | <5ms | winit 驱动 | — |
| 3 | `create_window` | `app.rs:174` | 20–50ms | 30–80ms | macOS native | 不可压 |
| 4 | wgpu Instance + Adapter + Device | `gpu.rs:54-70` | 5–15ms | **300–600ms** | 同步阻塞 | **B**：提前并行 |
| 5 | Surface configure | `gpu.rs:91` | <2ms | <5ms | — | — |
| 6 | `GlyphRenderer::new`（pipeline + WGSL） | `app.rs:199` | 5–30ms | 30–80ms | shader 编译 | **C**：pipeline cache |
| 7 | Atlas 纹理 + 1px 上传 | `app.rs:202-234` | <1ms | <2ms | — | F：起步缩成 1024² |
| 8 | **`Shaper::new` → `FontSystem::new()`** | `app.rs:260` / `shaping/lib.rs:138` | **50–300ms** | **150–500ms** | 字体扫描 | **A**：延迟/缩小 |
| 9 | `DocumentView::from_file`（同步 `read_to_string`） | `app.rs:286` | <5ms（小） / >100ms（大） | 同上 + IO | 同步 IO | **E**：mmap + 流式 |
| 10 | 第一帧 shape（无 cache） | redraw 路径 | 30–50ms（50 行） | 同 | shaping | E：限制可见行 |
| 11 | 第一帧 rasterize + atlas upload | swash | 30–80ms | 同 | swash | F：ASCII warm-up |
| 12 | 第一次 present（vsync） | wgpu | ~16ms | ~16ms | 必要 | — |

**串行总和**：warm ~150–500ms，cold ~600–1200ms。

**主要嫌疑**（占比从高到低）：
1. Step 8 `FontSystem::new` —— 单点最大头
2. Step 4 wgpu 初始化（cold）
3. Step 10+11 第一帧 shape & rasterize
4. Step 6 pipeline 编译

---

## 3. Phase 0：插桩量化（先决条件）

### 3.1 目标

**用真实数字驱动后续优先级**，避免照着估算盲目优化。

### 3.2 实现

无新依赖、可关闭、零运行期开销：

```rust
// crates/stdext/src/startup_trace.rs（新增）
use std::sync::OnceLock;
use std::time::Instant;

static ENABLED: OnceLock<bool> = OnceLock::new();
static T0: OnceLock<Instant> = OnceLock::new();

pub fn init() {
    ENABLED.get_or_init(|| std::env::var("EDITPLUS_TRACE_STARTUP").is_ok());
    T0.get_or_init(Instant::now);
}

#[inline]
pub fn enabled() -> bool { *ENABLED.get().unwrap_or(&false) }

#[macro_export]
macro_rules! trace_startup {
    ($label:expr) => {
        if $crate::startup_trace::enabled() {
            let t0 = $crate::startup_trace::t0();
            eprintln!("[startup] +{:>6.2}ms  {}", t0.elapsed().as_secs_f64() * 1000.0, $label);
        }
    };
}
```

### 3.3 插桩点（约 12 处）

| 位置 | 标签 |
|------|------|
| `main.rs` 进入 `main` | `main:enter` |
| `main.rs` 解析 CLI 后 | `main:cli-parsed` |
| `main.rs` `EventLoop::new` 后 | `main:event-loop-created` |
| `app.rs::resumed` 进入 | `resumed:enter` |
| `app.rs::init_window` `create_window` 前后 | `init_window:created` |
| `gpu.rs::create_gpu_context` adapter 拿到 | `gpu:adapter-ready` |
| `gpu.rs::create_gpu_context` device 拿到 | `gpu:device-ready` |
| `gpu.rs::create_gpu_context` surface configured | `gpu:configured` |
| `app.rs::init_text` `GlyphRenderer::new` 后 | `text:renderer-ready` |
| `shaping/lib.rs::Shaper::new` `FontSystem::new` 后 | `shaping:fontsystem-ready` |
| `app.rs::init_text` 全部完成 | `text:done` |
| `app.rs::load_file` 完成 | `file:loaded` |
| 第一次 `redraw_requested` 返回前 | `frame:first-presented` |

### 3.4 测量协议

```bash
# warm
EDITPLUS_TRACE_STARTUP=1 ./target/release/edit-plus --headless 2>&1   # x10 取中位数
EDITPLUS_TRACE_STARTUP=1 ./target/release/edit-plus README.md         # x10 GUI

# cold
sudo purge && EDITPLUS_TRACE_STARTUP=1 ./target/release/edit-plus --headless 2>&1  # x3
```

### 3.5 输出物

- 一张 `docs/perf/startup-baseline-2026-06-01.md`，含 warm/cold 各阶段中位数表。
- 后续每个 phase 完成后回填 `before/after` 列。

### 3.6 退出条件

- 拿到 warm × 10、cold × 3 的中位数表。
- 排序确认本计划 §4 的 A–E 优先级。

---

## 4. 优化方向（A–E + 附加项）

下述每项都是**独立可上线**的，可单独评估收益。

### 4.1 A — 延迟/缩小 FontSystem 加载

**问题**：`FontSystem::new()` 在 macOS 上扫所有系统字体，是单点最大头。

**方案**：

1. **首屏 minimal db**：`Shaper::new()` 改为 `FontSystem::new_with_locale_and_db`，初始 db 只塞我们要用的等宽字体（settings.font_family，常见 fallback）。
2. **后台补全**：第一帧 present 之后，在后台线程执行完整字体扫描，结果通过 channel 发回主线程合并到 `font_system`。
3. **fallback 兜底**：CJK/emoji 等命中后台扫描完成前的 frame 时，渲染成 tofu 块，扫描完成后自动 invalidate shape_cache 重新 shape。

**接口设计**：

```rust
// shaping crate
impl Shaper {
    pub fn new_minimal(family: &str) -> Result<Self, ShapeError>;  // 仅 family
    pub fn extend_with_system_fonts(&mut self);                    // 后台调用
}
```

**风险**：
- font fallback 行为在 minimal 阶段降级（不影响主用 ASCII/CJK 的常见路径，因为 monospace family 通常已含基本 CJK）。
- shape_cache 在扩库后需要 invalidate，要确保 invalidate 路径正确。

**预估收益**：warm -50~150ms，cold -100~400ms。

---

### 4.2 B — GPU 初始化提前并行

**问题**：wgpu adapter/device 请求是 cold 路径上的第二大头，且与 winit 窗口创建串行。

**方案**：

1. `App::new` 立即启动后台线程：
   ```rust
   let gpu_init = std::thread::spawn(|| {
       let instance = wgpu::Instance::new(...);
       let adapter = pollster::block_on(request_adapter(&instance, None));
       let (device, queue) = pollster::block_on(adapter.request_device(...));
       (instance, adapter, device, queue)
   });
   ```
2. `resumed` 拿到 window 后：
   - `surface = instance.create_surface(window)`（轻量）
   - `gpu_init.join()` 拿到 device/queue（多数情况已 ready）
   - configure surface 即可。

**前提验证**：wgpu 24 在 macOS Metal 上支持 surface-less adapter 请求（需要在 Phase 0 验证一次，加 `force_fallback_adapter=false` + `compatible_surface=None` 是否能拿到 high-perf adapter）。

**风险**：
- 如果 macOS Metal 后端要求 surface 才能选 high-perf adapter（不太可能但要确认），方案降级为「在 `App::new` 起线程做 instance 创建」。
- 线程 join 错误处理：若后台线程 panic，主线程要 fallback 同步初始化。

**预估收益**：cold -200~500ms（与窗口创建并行），warm -5~10ms。

---

### 4.3 C — Pipeline cache 持久化

**问题**：`GlyphRenderer::new` 每次启动都重新编译 WGSL → MSL。

**方案**：

1. 启用 `wgpu::PipelineCache`：
   - 缓存路径：`~/Library/Caches/edit-plus/pipeline-cache.bin`（macOS）/ `$XDG_CACHE_HOME/edit-plus/`。
   - 启动时读取 → 传给 `device.create_pipeline_cache`。
   - 退出（或定期）时写回。
2. 缓存 key 包含：wgpu version、adapter name、shader hash。版本不匹配时丢弃。

**前提验证**：Metal 后端在 wgpu 24 是否实质支持 pipeline cache 落盘（部分版本是 no-op）。Phase 0 跑一次确认。

**风险**：缓存损坏时不能让程序无法启动 → 读失败时静默 fallback。

**预估收益**：warm -5~25ms。如果 Metal 是 no-op，本项跳过。

---

### 4.4 D — 首屏「空窗口先呈现」

**问题**：用户感知的「启动速度」≈ 窗口可见时间，目前要等所有 init 完成才显示窗口（winit 默认 `visible=true` 但要等第一帧才有内容）。

**方案**：

1. `init_window` 完成后立即绘制「空 surface」一帧（背景色），而不是等 `init_text + load_file`。
2. `init_text` 与 `load_file` 推迟到第一帧 present 之后（在 `RedrawRequested` 中按状态机推进）：
   - state = `WindowReady` → 绘制空 surface
   - state = `TextReady` → 可以 shape 但还没文件
   - state = `FileReady` → 完整渲染
3. 大文件可以加一个微型「Loading…」提示在 D 中渲染（用 atlas 的 white pixel + 简单矩形即可，不需要等 shaper）。

**风险**：
- 状态机化 init 路径，`app.rs` 的复杂度上升，需要清晰的状态枚举。
- 如果第一帧空白时间过长（>100ms），观感反而更差 → 需要 Phase 0 数据支撑判断。

**预估收益**：「窗口可见」时间 -200~400ms（感知层），但**总耗时不变甚至略增**（多一次 present）。

---

### 4.5 E — 大文件 mmap + 流式首屏

**问题**：`DocumentView::from_file` 当前是 `read_to_string`，整文件全读 + UTF-8 校验，大文件直接 stall 主线程。

**方案**：

1. **mmap 读入**：用 `memmap2::Mmap` 替代 `read_to_string`，惰性页加载。
2. **首屏只 shape 可见行**：当前已经只 shape visible，但 `from_file` 还要算行号偏移。改为：
   - 流式扫第一屏需要的字节（按 `visible_rows × avg_line_bytes` 估算）。
   - 后续行号 index 放后台 build。
3. **edit 路径兼容**：第一次 edit 时把 mmap 转 owned buffer，避免 mmap 的写入复杂度。

**风险**：
- mmap 在文件被外部修改时行为复杂（可接受，因为编辑器都按打开瞬间快照）。
- gap_buffer 当前 API 与 `&[u8]` 接口的兼容（已有 `read_forward`/`read_backward`，影响小）。

**预估收益**：小文件无变化，大文件首屏 -50~300ms（取决于大小）。

---

### 4.6 附加项（小但便宜）

| 项 | 收益 | 复杂度 |
|----|------|-------|
| **F1**：atlas 起步 1024×1024，需要时再 grow 到 2048 | warm -<1ms（主要省内存） | 低 |
| **F2**：启动后立刻 raster ASCII 32–126 进 atlas | 第一帧 -10~30ms | 低 |
| **F3**：`pollster::block_on` 改 `block_on_local`（避免 thread spawn） | warm -<2ms | 低 |
| **F4**：`shape_cache` / `wrap_cache` 容量从启动配置读取，避免运行期 grow | <1ms | 低 |
| **F5**：debug build 用 `RUSTFLAGS=-Zthreads=8` 影响构建非运行——略 | — | — |

---

## 5. 阶段切分（按 plans 原子化原则）

**每个 phase 独立可合并、可回滚、不依赖未合的下游**。Phase 0 是先决条件，Phase 1–5 之间互不依赖（除 4 依赖 1 的状态机基础）。

### Phase 0 — 插桩与基线（先做）
- 输出：`crates/stdext/src/startup_trace.rs` + 12 个 trace 点
- 测量脚本：`scripts/measure_startup.sh`
- 基线文档：`docs/perf/startup-baseline-2026-06-01.md`
- 退出条件：拿到 warm × 10 + cold × 3 的数据表
- 工作量：~150 行代码 + 测量

### Phase 1 — A：FontSystem 延迟加载
- 修改：`crates/shaping/src/lib.rs` 新增 `new_minimal` / `extend_with_system_fonts`
- 修改：`crates/app/src/app.rs::init_text` 调用 minimal 版
- 新增：后台扩库线程 + invalidate shape_cache 钩子
- 测试：CJK/emoji fallback 在扩库前后行为一致
- 工作量：中

### Phase 2 — B：GPU 初始化并行
- 前置验证：surface-less adapter 在 Metal 可用
- 修改：`App::new` 启线程；`gpu.rs` 拆 `create_instance_and_device` / `attach_surface`
- 测试：线程 panic fallback、adapter 失败 fallback
- 工作量：中

### Phase 3 — C：Pipeline cache（条件性）
- 前置验证：wgpu 24 Metal pipeline cache 落盘有效
- 若有效 → 实现；若无效 → 关闭 phase，写报告说明
- 工作量：低

### Phase 4 — D：首屏空窗口（依赖 Phase 0 数据决定要不要做）
- 仅当 Phase 0 数据显示「窗口可见时间 > 200ms」才上
- 修改：引入 `InitState` 枚举，`app.rs` 状态机化 init 流程
- 工作量：中高

### Phase 5 — E：大文件 mmap + 流式首屏
- 前置：定义大文件阈值（>1 MB 触发 mmap 路径？数据决定）
- 修改：`DocumentView::from_file` 双路径
- 测试：编辑路径 mmap → owned 转换正确
- 工作量：中

### Phase 6 — F：附加项打包
- F1/F2/F3/F4 一起做
- 工作量：低

---

## 6. 风险与回滚

| 风险 | 缓解 |
|------|------|
| Phase 1 minimal db 漏字体导致 fallback 异常 | 维持 system fallback chain，扩库失败时 fallback 同步加载 |
| Phase 2 surface-less adapter 在某些机型不可用 | 加 `cfg!` + 运行期 detect，失败时降级同步 |
| Phase 3 pipeline cache 损坏 | 读失败静默丢弃，写失败 log 但不阻塞 |
| Phase 4 第一帧空白时间反而被放大 | Phase 0 数据卡 gate；空白超过阈值则关闭本 phase |
| Phase 5 mmap 与 edit 路径冲突 | 第一次写入即 promote 到 owned，单元测试覆盖 |
| 总体：插桩残留进生产 | `EDITPLUS_TRACE_STARTUP` 默认关闭 + macro 在未启用时零开销 |

---

## 7. 验收

- 每个 phase 落地后，重跑 §3.4 测量协议，把 before/after 写进 `docs/perf/startup-baseline-2026-06-01.md`。
- 若某 phase 实测收益 < 5ms 且改动复杂度高，考虑回滚而不是合并。
- 全部完成后，更新 §1.2 的目标表为实测达成值。
