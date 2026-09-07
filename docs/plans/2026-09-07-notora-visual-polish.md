# Notora 界面整体优化

目标：保留暖灰配色和固定三栏，减少外壳视觉重量，改善编辑空间、导航层级与操作可发现性。

架构：产品状态只在 notora-app 映射为 UI 纯数据输入；共享组件负责几何、绘制和命中。正文视口仍使用共享 runtime 的同一套绘制与输入坐标。

约束：全中文；沿用用户已保存的栏宽和主题；脑图使用完整画布；不改变笔记存储与自动保存；每个子任务不超过三个实现文件；重大修改运行 `./scripts/verify.sh`。按 writing-plans / subagent-driven-development 流程实施，保持未提交改动供用户审阅。

## 子任务与验收

共享几何补充：`ui/layout.rs` 的 `reading_content_inset` 统一计算标题、属性、工具栏和正文的 32px 留白与 760px 最大阅读宽度；正文变更同时更新 `appkit-shell/editor_runtime/mod.rs` 的真实 IME 坐标断言。


- [x] 1. 编辑器头部（`shell/layout.rs`、`editor_pane.rs`、`ui/widgets/editor_header.rs`）：工作区头部 84px、工具栏 36px；标题 22px；标题和属性左侧 32px 对齐；默认显示修改时间和保存状态，创建时间保留在悬停提示。属性行保留可交互工作区与标签入口，省去冗长前缀。验证小窗口、长标题、按钮命中、元信息裁剪与悬停。
- [x] 2. 笔记列表（`ui/widgets/virtual_card_list/mod.rs`、`layout.rs`）：普通项不画独立背景，选中/悬停项保留圆角浅底；卡片间距 4px、垂直内边距 10px、元信息间距 8px，保持两行摘要与虚拟滚动。运行现有组件测试。
- [x] 3. 工具栏组件（`ui/widgets/editor_toolbar.rs`）：识别已有纯数据 groups，组内间距 2px、组间距 12px、按钮 28px、左右留白 32px；绘制、命中、tooltip、无障碍与溢出计算共用布局；窄栏所有动作可通过更多菜单访问。先测试多组与窄栏溢出。
- [x] 4. 导航层次（`ui/widgets/tree_list/mod.rs`、`layout.rs`、`notora-app/render.rs`）：为树列表加入可选分组间距纯数据，不改变默认调用方；Notora 工作区之后、辅助文件区域之前留 12px；统一侧栏文字密度，工具栏按语义构造多个组。先验证分组间距与行内新建/滚动命中一致。
- [x] 5. 正文阅读宽度（`appkit-shell/editor_runtime/editor_painter.rs`）：非画布视图 32px 水平留白，最大正文宽度 760px 居中，顶部留白 16px；小尺寸下矩形不越界；脑图完全沿用原矩形。先验证 DPI、小尺寸、居中、画布边界，运行 runtime 输入相关测试。
- [x] 6. 脑图视图入口（最多三个实现文件）：接入现有画布缩放/复位动作，在脑图工具栏提供缩放与适应窗口入口；复用既有视口状态，不引入新坐标系统。验证命令路由及正文模式不受影响。
- [x] 7. 综合验证：`cargo fmt --all`、`./scripts/verify.sh`、`cargo build -p notora-app`；用隔离配置及样例工作区运行新二进制，检查普通笔记、脑图、窄窗口和更多菜单；代码审查后完成。

## 审查跟进

- 窄栏工具栏已有的更多按钮缺少产品菜单接线：在 `editor_pane.rs` 补齐真实菜单、命令回传及生命周期。
- 树分组最初逐行扫描前缀导致平方复杂度：改为一次前缀计数，保留插入与滚动坐标。
- 头部 compact 判断原先使用 480px 窗口高度阈值，导致总处于紧凑模式并重复显示删除入口：统一使用头部自身的几何规则，已完成失败复现与修复验证。
- 脑图提供三个图标按钮，适应窗口使用静态提示，避免缓存提示展示过期缩放比例。

## 验证记录

基线与最终命令输出分别记录于 `/tmp/notora-baseline.log`、`/tmp/notora-verify.log`；截图使用 computer-use 读取实际窗口。

- 基线：Notora 356 项库测试通过。
- 首轮：Notora 359 项库测试通过；appkit-shell 516 项库测试通过；完整脚本通过架构/格式/clippy，后在 sync 本机 mock server bind 遇沙箱权限限制，已单独申请沙箱外测试。
- 运行预览：隔离配置 `/tmp/notora-visual-preview/config` 与生成的样例工作区，已检查正文、脑图放大、适应窗口和缩窄窗口。

- 更多菜单：6 项新增回归测试先失败再通过，editor_pane 31 项通过；Retina 窄窗口实机展开、方向键选中及回车执行隐藏编号列表命令通过。
- 代码审查：更多菜单缺失与树列表平方复杂度两个问题均已修复并复核关闭。当前最小窗口尺寸可容纳全部菜单项；未来若增加命令数量或降低窗口最小高度，需同步调整通用菜单容量策略。

- 最终完整验证：`./scripts/verify.sh` 退出码 0，输出 `All checks passed! Baseline is trusted.`，架构、格式、Clippy、工作区测试及文档测试全部通过。其中 Notora 365、appkit-shell 516、Markdown 1282、sync 27 项库测试通过；完整日志 `/tmp/notora-verify.log`。
- 最终构建：`cargo build -p notora-app` 退出码 0，日志 `/tmp/notora-build.log`；`git diff --check` 通过。
- 最终预览截图：`/Users/dan/.codex/visualizations/2026/09/07/01a07b0f-b3aa-78b1-815a-942025e8b456/notora-after.png`。改动保持未提交。

## 用户反馈跟进：标题留白与文件页操作

- 标题子任务（两个实现文件）：标题输入框左右各 8px 内边距、高度 36px，输入框向外扩展以保持文字与正文对齐；头部相应调整为 92px。DPI 1/2 的光标、内边距、行高及元信息不重叠回归先失败再通过。
- 文件页子任务（两个实现文件）：打开按钮显示文件夹图标；复用带加号的新建分体按钮，主按钮创建 Markdown，下拉支持 TXT、脑图、Markdown。文件页不显示仅工作区支持的加密笔记入口，未设置工作区也能创建未命名文件；空态文案同步为文件。
- 验证：标题组件 13 项、Notora 库 367 项通过；完整 `./scripts/verify.sh` 退出码 0，日志 `/tmp/notora-controls-verify.log`；应用构建通过，日志 `/tmp/notora-controls-build.log`。
- 实机：已检查标题编辑态、打开图标、新建文件按钮与下拉菜单；主按钮创建 Markdown、菜单创建 TXT 成功。改动未提交。

### 标题垂直布局复核

用户确认剩余问题为上下留白及位置不协调。复现发现：92px 头部扣除属性行后仅剩 64px，原先 36px 标题 + 4px 行间距 + 24px 元信息恰好占满，标题容器顶部仍为 0。重新分配为明确的 8px 顶部留白、32px 标题行、24px 元信息行，去掉额外行间距，总高度不变。标题文字下移约 6px，元信息保持原位置。

DPI 1/2 布局回归先失败再通过；标题组件 14 项、Notora 库 367 项通过，应用构建、格式及 diff 检查通过。独立样例预览已检查标题静态和聚焦状态；截图 `/Users/dan/.codex/visualizations/2026/09/07/01a07b0f-b3aa-78b1-815a-942025e8b456/notora-title-position.png`。

### 文件页操作简化

清空按钮增加关闭图标；工作区与文件页的主按钮文案统一为“新建”，宽度同步缩至 96px。文件页主按钮默认创建纯文本，工作区仍默认 Markdown，类型菜单保留显式格式选择。默认类型与缺失图标回归先失败再通过；55 项 render 测试及应用构建通过。

## 提交前验收

用户已要求提交并推送远端 main。最终代码复核无阻断问题；新增测试的 Clippy 初始化写法已修正。完整 `./scripts/verify.sh` 退出码 0，包含 Notora 368 项库测试、集成测试、工作区测试及文档测试；日志 `/tmp/notora-prepush-verify.log`。
