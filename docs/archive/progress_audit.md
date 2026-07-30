# edit+ 进度审计报告

审计日期：2026-05-29
审计范围：plans.md 全部 12 个阶段 + auxiliaries（样本、手册、bench、文档）
审计依据：实际代码 / 测试 / 构建结果 / `docs/audit_fix.md` / `docs/audit_fix_v2.md`

---

## 0. 一句话结论

**核心层（stage 0–2）已稳：** 代码、测试、bench、静态分发、零警告全部到位。
**渲染/文件层（stage 3–5）"形似神不至"：** 框架搭起、smoke 通过，但
N1（app 层读文件回退到 `std::fs::read_to_string`）、N2（每帧无条件重 shape + 持续 redraw）、
N3（atlas 是 1×1 白像素，根本没光栅化 glyph）三个 v2 中已记录的 P0 问题**仍未修复**，
导致 stage 4/5 的核心验收（golden image SSIM、RSS、idle CPU）实际失效。
**stage 6–12 全部未开始。**

---

## 1. 阶段进度总览

| 阶段 | 主题 | 状态 | 关键证据 |
|---|---|---|---|
| 0 | 工程骨架 | ✅ 完成 | `cargo build --workspace` 通过；clippy/fmt 0 warning；stdext 17 测试 + lsh 2 测试全绿 |
| 1 | core crate 抽取 | ✅ 完成 | `cargo test -p edit-plus-core` 96 passed / 1 ignored；`rg "use crate::(framebuffer\|cell\|vt\|tui\|input\|sys::)" crates/core/src/` 无匹配 |
| 2 | measurement 像素化 | ✅ 完成 | `crates/core/src/unicode/measurement.rs:70` `GraphemeAdvance` trait + `TerminalAdvance`(L88) + `PixelAdvance`(L115)；6 个 pixel_* 测试全过；`grep "dyn GraphemeAdvance"` = 0 |
| 3 | winit + wgpu 空窗口 | 🟡 部分 | smoke + resize 测试过；**缺**：启动耗时/idle CPU 实测、设备丢失/IME 占位、外接显示器场景 |
| 4 | cosmic-text + 静态渲染 | 🟡 部分 | shaping/render API 与 §4.3/§4.4 一致；shape/atlas 单元测试齐全；**缺**：`render_hello_to_png` golden + SSIM、bench 阈值断言、**N3 未修：屏幕渲染的是白矩形不是文字** |
| 5 | 只读显示一个文件 | 🟡 部分 | `core/src/file.rs` 零拷贝路径已重写，22 个样本齐；`DocumentView` + `Viewport` + `scan_line_offsets` 就位；**缺**：`bench_open_50mb_*` / `bench_scroll_60s_60fps` / RSS 实测；**N1 未修：app 层没用 core::file::load_file** |
| 6 | 键盘输入 + 编辑 | ❌ 未开始 | `app.rs` 仅响应 Escape；无 cursor / insert / IME；`text_buffer.rs.deferred` 3189 行未启用 |
| 7 | 选择 + 剪贴板 + Undo | ❌ 未开始 | 无 selection state；`Cargo.toml` 无 `arboard` |
| 8 | 文件 IO 闭环 | ❌ 未开始 | 无 save / dirty / dialog；`DocumentView::from_file` 还在用 `std::fs::read_to_string` |
| 9 | 多 buffer + Tab UI | ❌ 未开始 | `App` 持有单一 `Option<DocumentView>`；无 TabBar |
| 10 | 搜索（SIMD） | ❌ 未开始 | 仅 `core::simd::memchr2` 工具，无搜索 API/UI |
| 11 | 替换 + ICU 正则 | ❌ 未开始 | `core/src/icu.rs` 仅动态加载框架；`icu.rs.deferred` 1372 行未启用；regex/replace 完全未接 |
| 12 | 性能基线 + 优化 | ❌ 未开始 | 无 `docs/perf_baseline.{md,json}`、`docs/perf_notes.md`；无 CI bench 守门 |

---

## 2. Auxiliary 资产对照

| 项 | plans.md 要求 | 现状 |
|---|---|---|
| 样本语料 `assets/samples/` | §9 22 项 | ✅ 22 个文件齐（含 SHA256SUMS） |
| 生成脚本 `scripts/gen_samples.sh` | 幂等、跨平台 | ✅ 已存在 |
| 手动测试协议 `docs/manual_test_protocol.md` | §10 必备 | ❌ **缺失** |
| 性能基线 `docs/perf_baseline.{md,json}` | 阶段 12 | ❌ 缺失 |
| `docs/perf_notes.md` | 阶段 12 | ❌ 缺失 |
| `tests/golden/hello_edit_plus.png` | 阶段 4 | ❌ 缺失 |
| audit 记录 | — | ✅ `docs/audit_fix.md` + `docs/audit_fix_v2.md` |
| plans.md §7 抄码 checklist | 与现实一致 | ❌ N9：与现实严重不符 |

---

## 3. 已识别但**未修复**的关键问题（按 audit_fix_v2.md）

下表把 v2 报告里的待办项汇总；本审计复跑 v2 §5.2 验收脚本，结论一致，未见改善。

### 3.1 P0（阻断 stage 4/5 真验收）

| 项 | 位置 | 影响 | 状态 |
|---|---|---|---|
| **N1** app 层重新引入 `std::fs::read_to_string` + `Vec<String>` | `crates/app/src/document_view.rs` | 50 MB 文件 RSS × 3，违反 §5 RSS<150 MB；丢弃行尾符违反 stage 8 EOL 保留；与 §4.1 设计相悖 | ❌ |
| **N2** `about_to_wait` 无条件 `request_redraw` + 每帧重 shape 全部行 | `crates/app/src/app.rs:452-456` / `app.rs:243-299` | idle CPU 远超 stage 3 < 0.5% 门槛；cosmic-text line cache 失效 | ❌ |
| **N3** atlas 仅 1×1 白像素，未光栅化 glyph | `crates/app/src/app.rs:130-158` / 271-295 | 屏幕显示白矩形条；阶段 4 golden SSIM ≥ 0.99 不可能通过 | ❌ |
| **M2** `Shaper::shape` 每次 `Buffer::new` | `crates/shaping/src/lib.rs` | cosmic-text line cache 完全不工作 | ❌ |

### 3.2 P1（次级正确性 / 资源）

| 项 | 位置 | 现状 |
|---|---|---|
| **M1** shaping cache key 仍是单纯 String，未含 (cluster, font_size, attrs_hash) | `crates/shaping/src/lib.rs:68-112` | 🟡 LRU 已上 hashlink，但 key 复合化未做 |
| **M3** `Default for Shaper` 含 `unwrap_or(...)` 死分支 | `crates/shaping/src/lib.rs` | ❌ |
| **M4** atlas LRU 仍走 `Vec + retain + remove(0)` 的 O(N) | `crates/render/src/lib.rs` | ❌ |
| **M5** atlas oversized 重复创建 + 丢弃 page | `crates/render/src/lib.rs` | ❌ |
| **M6** `app::init_window` 4 处 `expect("...")` | `crates/app/src/app.rs` | ❌ |
| **M7** `gpu.rs::create_gpu_context` 写好但 `app.rs::init_window` 没切过去 | `crates/app/src/app.rs` / `gpu.rs` | 🟡 半成品 |
| **M8** `mod sys` 缺 `icu_detect_renaming_suffix` / `icu_add_renaming_suffix` | `crates/core/src/icu.rs:609,619` | 🟡 macOS 不影响编译；Linux 自定义 ICU 后缀场景立即 broken |

### 3.3 P2（小修）

| 项 | 位置 | 现状 |
|---|---|---|
| **N4** icu sys 缺 2 个函数 | `crates/core/src/icu.rs` | ❌（同 M8） |
| **N5** `mod sys` 缩进 fmt 不一致 | `crates/core/src/icu.rs:14-67` | ❌（`cargo fmt` 会自动改） |
| **N6** `detect_line_ending` 与 `scan_line_endings` 双轨 | `crates/core/src/file.rs:54 vs 161` | ❌ |
| **N7** `Viewport::scroll_up` 没 clamp | `crates/app/src/viewport.rs:49` | ❌ |
| **N8** `hashlink` 没进 `workspace.dependencies` | `crates/shaping/Cargo.toml` | ❌ |
| **N9** plans.md §7 抄码 checklist 与现实严重不符 | `plans.md:769-799` | ❌ |

---

## 4. 本次审计新发现 / 补充观察

### 4.1 plans.md §7 checklist 错误（接续 N9）

实际状态如下，plans.md §7 表格需要更新：

| 文件 | plans.md 标 | 实际 |
|---|---|---|
| `cell.rs` | `n` | ✅ 已抄（应改 `y`，无终端依赖） |
| `lsh/cache.rs` / `lsh/highlighter.rs` / `lsh/definitions.rs` | `y` | ❌ 全部 deferred 到 stage 13 |
| `core/src/file.rs` | 不在表 | 自研，应增列"自研文件"小节 |
| `app/src/{cli,gpu,viewport,document_view}.rs` | 不在表 | 自研，应增列 |

### 4.2 测试名称对齐情况

plans.md 阶段 5 要求的若干具名测试**未按字面落地**：

- `core::buffer::tests::open_file_via_path` —— 不存在；功能在 `core::file::tests::load_file_*` 里覆盖了，但与 plans.md 命名不符。
- `core::buffer::tests::handle_bom` —— 同上，落在 `core::file::tests` 里。
- `viewport_only_shapes_visible_lines` —— `crates/app/src/document_view.rs:364` 命名为 `viewport_only_returns_visible_lines`，且**只断言 visible_line 数量，不断言 shape 调用次数**——验证强度不够（stage 5 验收要求"shape 调用 ≤ 视口行 × 2"）。

建议：要么按 plans.md 重命名 + 补桩计数，要么更新 plans.md 的测试名清单（保留语义即可）。

### 4.3 stage 1 一处 TODO

`crates/core/src/unicode/measurement.rs:394` 含 `// TODO: handle cross-chunk clusters properly`——
跨 chunk 的 grapheme cluster 走 fallback 到 UCD 表宽度。stage 2 内可接受；stage 5+ 大文件 + 滚动场景需要补回归测试。

### 4.4 `text_buffer.rs.deferred` / `icu.rs.deferred` 风险

两份 deferred 文件分别是 3189 行 / 1372 行，依赖大量已被删除的 `terminal_stubs / HighlightKind / Clipboard / Language` 等。stage 6/11 真正接入时不能直接 `mv .deferred → .rs`，需要把上述依赖**重新映射到 GUI 等价物**——这是隐藏在 plans.md 阶段 6/11 估时之外的"暗工作量"，应在阶段计划里显式标记。

---

## 5. 推荐处理顺序

> 不追加新功能，先把 stage 4/5 真正达标，再开 stage 6。

### 5.1 立即（P0，阻断 stage 4/5 验收）

1. **N3 + M2** 真 atlas 接入（swash 光栅化 → R8Unorm atlas → fragment shader 改 alpha 采样） + Shaper 复用 cosmic-text Buffer。`crates/shaping`、`crates/render`、`crates/app/src/app.rs` 三处协同改。
2. **N1** `DocumentView` 改持 `GapBuffer + LineIndex`，`from_file` 直接调 `core::file::load_file`。
3. **N2** 引入 `needs_redraw` 脏标记 + 行内容 hash → shape cache。

### 5.2 之后（P1）

- M6（错误处理切到 `gpu.rs::create_gpu_context`）
- M4 + M5（atlas LRU O(1) + oversized 缓存）
- M1（shaping cache 复合 key）
- M3（删 `Default for Shaper` 死分支）

### 5.3 收尾（P2 + 文档）

- N4–N9 全部修
- 写 `docs/manual_test_protocol.md`（至少阶段 5 节先到位）
- 落 `bench_open_50mb_ascii` / `bench_open_50mb_cjk` / `bench_scroll_60s_60fps` + 阈值断言
- 准备 `tests/golden/hello_edit_plus.png` + SSIM 比对工具
- plans.md §7 checklist 与现实对齐

### 5.4 真正进入 stage 6 之前

- 重跑 v2 §5.2 全部门槛
- 在 plans.md 阶段 6 章节里**显式标注** "deferred 文件依赖重写工作量"（stub 类型重映射到 GUI 等价物的清单）

---

## 6. 现状打分（个人估）

| 维度 | 评分（10 分制）| 备注 |
|---|---|---|
| Stage 0–2（核心层）落地质量 | 9 | 代码工整、测试稠密、零警告 |
| Stage 3–5 框架完整度 | 6 | 形态对，但 N1/N2/N3 让验收悬空 |
| Stage 6–12 进度 | 0 | 未启动 |
| 文档/手册 | 4 | 计划详尽；手动协议 / 性能基线 / golden 缺位 |
| 流程纪律（CLAUDE.md §6/§7）| 5 | v2 已识别 P0 仍未修，明显在"叠补丁推后"——违反 §7 |

整体进度：**workspace + core 基础（~30%）已稳，渲染/文件层"看起来能跑"但本质未完成（~10%），其余 60% 未启动。** 当前最大风险不是工作量，而是 **stage 4/5 的"伪完工"**——若不回头修 N1/N2/N3，所有后续阶段的性能/正确性验收都是建立在沙地上。
