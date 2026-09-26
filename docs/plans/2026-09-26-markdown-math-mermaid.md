# Markdown 公式与 Mermaid 实施计划

> **For agentic workers:** 使用 subagent-driven-development；开发由 gpt-6-sol 执行，主代理负责接口、审查和验收。每个子任务修改最多三个文件，超过时先继续拆分。未经编译验证不提交。

**Goal:** 在原生 Markdown 编辑器完整显示并可编辑数学公式和 Mermaid 图表。

**Architecture:** 数学/图表引擎 → SVG → resvg RGBA → ui 纯数据图像命令 → appkit-shell GPU；Markdown 负责排版及源码映射。

**Tech Stack:** Rust 1.93 基线、pulldown-cmark 0.13、RaTeX、mermaid-rs-renderer、resvg、现有 wgpu。

## 实施记录

- A1/A2：完成。最终 `EmbeddedImage { image: Arc<RasterImage>, baseline: f32 }`；引擎、错误回退、缓存和尺寸单位测试通过。独立审查发现的数学 `pt` 尺寸问题已先复现，再修复为 72 DPI；Mermaid 保持 96 DPI。
- B：完成。UI 图像协议、独立 RGBA 图集与两产品帧入口已接通。真实 GPU 像素读回测试通过，覆盖线性和 sRGB 输出目标、预乘半透明颜色；图集资源不足时显示可读提示。
- C：完成。解析、真实 advance、基线/行高、列表/表格、增量布局和源码展开均已接通。1435 项 Markdown 单元测试及集成测试通过；公开接口黑盒验收 10/10 通过。完整 Mermaid 错误源码和纯选区变化问题均先补失败测试，再修复。
- 独立审查未发现剩余 P1/P2。严格 Clippy 所需的两处既有循环改写保持行为等价，相关命令与表格测试通过。
- 验收样例：`docs/math-mermaid-examples.md`。格式检查、工作区编译以及完整 `./scripts/verify.sh` 均通过。完整验证在沙箱外运行，退出码 0，最终输出 `All checks passed! Baseline is trusted.`；首次沙箱内运行的同步测试因无法绑定本地模拟服务器端口而失败。
- 本机锁屏，未做应用界面截图验收；真实 GPU 读回及 Markdown DrawList 几何/像素验收已完成，不将这些测试描述为界面截图。

## Global Constraints

- 全程中文；遵守根 AGENTS.md，ui 不依赖 app 状态。
- `$…$`、`$$…$$`、mermaid 围栏都在范围内；其他代码保持原样。
- 先测试再实现；错误保留源码；缓存有界；不能把占位符当已实现渲染。
- 单个修改子任务不超过三个文件；大阶段必须继续拆分。
- 编译通过才提交；最终执行 `./scripts/verify.sh`。

## 阶段 A：引擎适配（Sol）

### A1：依赖与适配器

文件：`crates/markdown/Cargo.toml`、`crates/markdown/src/embedded.rs`、`crates/markdown/src/lib.rs`。Cargo.lock 作为随后依赖锁定子任务。

接口：`EmbeddedKind::{InlineMath, DisplayMath, Mermaid}`；`EmbeddedRequest` 包含源码、字号和前景/背景；`render_embedded` 返回 `EmbeddedImage { image: Arc<RasterImage>, baseline: f32 }`，图像提供宽高。错误返回有描述的 Result，不能 panic。图像类型统一使用 `ui::core::paint::RasterImage`。

- [x] 先写实际引擎测试：分式、根号/上下标、矩阵；流程图、时序图、中文标签；非法输入。
- [x] 确认测试先失败，再接入引擎与 resvg；数学 SVG 嵌入字形。检查非透明像素，不只检查字符串。
- [x] 缓存按完整请求区分，失败也缓存；为缓存命中和颜色/字号变化写测试。像素面积与源码长度有明确边界。
- [x] `cargo test -p textora-markdown embedded`；记录依赖版本/许可及结果。

### A2：锁定依赖

文件：`Cargo.lock`。验证可用版本与 Rust 基线；`cargo check -p textora-markdown`。

## 阶段 B：通用图像绘制（Sol）

### B1：UI 图像协议

文件：`crates/ui/src/core/paint.rs`（如需独立模块最多再两个）。

接口：不可变共享 `RasterImage`（唯一稳定 id、宽高、预乘 RGBA，构造时验证长度及溢出），`DrawCmd::Image { image: Arc<RasterImage>, rect: Rect }`；`DrawList::image` 正确应用 offset。

- [x] 测试像素验证、共享身份、offset。
- [x] 实现纯数据协议；更新穷尽匹配使用处，额外修改按最多三文件另分子任务。

### B2：GPU 资源与绘制

首先阅读 `crates/appkit-shell/src/render_state.rs`、`paint_backend.rs`、`text_rasterize.rs` 和 `crates/render/src/lib.rs`。根据现有单纹理顶点管线选择最小可靠 RGBA 图像路径，保持字形 gamma、透明混合、帧缓存有效和绘制顺序。分为资源、后端命令、着色器/上传三个最多三文件子任务；实施前报告明确文件分配。

- [x] 测试 RGBA 上传/颜色正确、透明像素、缩放后 UV、四边裁剪及完全不可见裁剪。
- [x] 图像按稳定 id 缓存，GPU 限制不允许静默丢图；资源有界且帧内不可被提前复用。
- [x] `cargo test -p textora-ui`、`cargo test -p textora-appkit-shell`（以实际 Cargo 包名为准）、相关 render 测试及编译。

## 阶段 C：Markdown 集成（Sol）

### C1：语义与源码

文件：`crates/markdown/src/parser.rs`、`builder.rs`、`edit.rs`（若只需两个则不动第三个）。启用 math 事件并保留字节范围；语义样式/块携带源码；沿用投影 builder。测试 `$x^2$`、多行 `$$`、行内代码、普通代码围栏、转义 `$`、相邻中文以及 Mermaid 原始范围。

### C2：行内与块排版

涉及 `layout/block.rs`、`layout/shaping.rs`、新 `layout/embedded.rs`；types/mod 接线若超出三个文件另为 C3。保留现有公开 layout struct 兼容性，优先 sidecar 或集中类型扩展。公式真实宽度参与换行，baseline 与最大 ascent/descent 决定行高；图表按可用宽度缩放。禁止按源码字符数预留宽度。

### C3：LazyLayout/投影状态

文件：`layout/types.rs`、`layout/mod.rs`、`layout/reconcile.rs`（按实际需要缩减）。保证首次 materialize、滚动重建、增量编辑与旧缓存失效一致；不得丢掉嵌入元素 sidecar。测试后续段落 y 坐标和源字节/视觉映射。

### C4：绘制与编辑展开

文件：`crates/markdown/src/render.rs`、`view.rs`、必要的新测试文件。通过图像命令绘制；光标/选区进入元素后展开源码，离开恢复预览。测试点击、选择、复制、编辑、撤销，以及非法语法回退。

## 阶段 D：集成审查与验证

- [x] 独立审查 diff，检查真实绘制链路、projection、行内布局和主题缓存；发现问题先复现后交 Sol 修复。
- [x] 添加可人工打开的 Markdown 验收样例到 `docs/`，包含公式、图表及失败例。
- [x] `cargo fmt --all -- --check`、`cargo check --workspace`、`./scripts/verify.sh`。
- [ ] 应用界面截图：本机锁屏，未执行。已用真实 GPU 读回及公开 Markdown 渲染/编辑接口测试补充验证；引擎限制记录在规格与本实施记录中。
