# Markdown 块间距统一实施计划

> 使用 subagent-driven-development 执行，子 agent 固定使用用户指定的 gpt-5.6-sol。逐项补充测试、实现、自审和复审。

**目标：** 实施已确认的块间距设计，修复分割线不对称及缩放差异，并保持编辑和惰性布局一致。

**架构：** Markdown 布局层统一解析相邻边界，块内容排版不包含外距。全文放置与已定位块精排使用明确的不同入口，共用内容排版；像素几何用于绘制和源码定位。

**技术栈：** Rust、textora-markdown、现有 ui/shaping 渲染和测试。

## 全局约束

- 遵守 AGENTS.md，全程中文，保留已有未提交的 UI 修改，不提交混合变更。
- 正文与分割线双向边界均为 max(paragraph_spacing, rule_spacing)。
- 真实空段落保留；外距不跨容器合并；公共 LaidOutBlock 字段形状不变。
- 源码态分割线仍保持分割线间距语义；固定逻辑尺寸只缩放一次。
- 不增加 ui 对 app 的依赖，不退化为每次输入全篇精排。
- 每个实现子任务控制在三个文件以内；核心模块拆为规则、放置和惰性布局三个子任务。

## 子任务与进度

### A. 行为测试（sol 测试 agent）
- 文件：新增 crates/markdown/tests/block_spacing.rs。
- [x] 先运行失败测试：正文/分割线、连续分割线、标题和列表两侧、额外空行、全文与物化一致。
- [x] 覆盖不同字号与行高、LF/CRLF、文首文末；非目标组合保存现有语义。
- [x] 实现后重跑并记录结果。

### B1. 统一规则（sol 核心 agent）
- 文件：layout/context.rs；必要时新增 layout/spacing.rs 并在 layout/mod.rs 注册。
- [x] 使用枚举表达块间距语义，集中正文、标题、列表和分割线的相邻规则。
- [x] 增加规则测试，删除独立 bool 间距状态和镜像逆向补偿。

### B2. 内容排版与放置（同一 sol 核心 agent 顺序执行）
- 文件：layout/block.rs、layout/context.rs。
- [x] layout_block 负责外部边界与块流放置，独立入口 layout_block_at_content_origin 只排已定位内容。
- [x] 所有块不再各自推进外距；容器内部隔离间距上下文，文首文末显式保留边界策略。
- [x] 分割线 rect.h 仅为 rule_thickness；源码态仍按分割线边界处理。

### C. DPI（sol 样式 agent）
- 文件：style.rs、view.rs、view_preedit.rs（必要的调用方变化作为独立后续子任务）。
- [x] 在生产样式构建路径补明确 DPI 解析，保留旧公共构造 API 的 1× 语义。
- [x] 测试 1×/1.5×/2× 下固定尺寸和比例尺寸，避免双重缩放。

### D. 惰性布局接入（主 agent）
- 文件：layout/types.rs 及需要的同模块测试文件。
- [x] 已有 content y 的物化/精排调用 layout_block_at_content_origin，不恢复先前物化块状态或预扣入口间距。
- [x] 验证类型变更、插删、激活收起、缓存淘汰后几何与冷启动一致。

### E. 绘制与交互（主 agent，D 完成后）
- 文件：render.rs；view.rs 的独立测试段（待 C 交还）；必要时源码投影文件作为独立子任务。
- [x] 分割线直接绘制内容几何；调试留白读取真实相邻位置，删除重复公式。
- [x] 通过既有结构间距中点归属规则保留分割线点选，验证真实空段落不被遮蔽。
- [x] 更新旧块高测试为用户可见的距离断言。

### F. 复审与完整验证
- [x] sol 独立审查本次差异，修复重要问题后针对性复测。
- [x] cargo test -p textora-markdown；cargo fmt 检查；./scripts/verify.sh。
- [x] 运行已有编辑性能基准的适当样例，记录局部编辑性能与限制。
- [x] 更新设计实施状态、测试证据和本计划进度。

## 验证记录

- 修改前 Markdown 单元基线：1307 项通过。
- 首轮独立间距测试：9 项中 6 项按预期失败；后续修正容器外框测量并补引用内部用例，共 10 项通过。
- 最终调试视图修复后：1333 单元测试、40 集成测试通过。独立复审确认稀疏物化调试边界问题已解决，无剩余重要问题。
- 首次 `./scripts/verify.sh` 的架构、格式、Clippy 和应用层测试通过；工作区测试中的同步模块有 12 项因沙箱禁止本机模拟服务器绑定端口而失败（`PermissionDenied`）。获自动审批后在沙箱外重跑完整验证，退出码 0，输出 `All checks passed! Baseline is trusted.`，同步测试也全部通过。完整日志位于 `/private/tmp/textora-spacing-verify-unrestricted.log`。
- 最终架构检查、`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、应用层测试及工作区单元、集成和文档测试均通过；保留仓库既有的忽略测试。`git diff --check` 通过。
- 同环境性能对照：从 HEAD 提取修改前 Markdown 源码至临时目录，共用当前依赖及工具链；与当前优化基准二进制顺序运行，分别使用独立 CRITERION_HOME。22KiB mixed_document_single_key，20 样本，2 秒测量，修改前区间 944.88–951.75µs，中心 947.97µs；修改后区间 908.93–918.86µs，中心 913.65µs。
- 该基准测量现有增强、解析和完整布局流水线，不能替代整机交互延迟；局部编辑复用、视口内精排由现有行为测试约束。并行全库测试期间的初次结果未作为性能结论。
