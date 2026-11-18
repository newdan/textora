# notora 界面主题 P0 实施计划

> **执行方式：** 在当前工作流按子任务逐项实施和复核。已获授权实施 P0，P1 仅保留候选；本计划不重新请求已明确的执行决策。

日期：2026-11-02  
状态：P0 已完成，P1 仅候选  
规格：`designs/notora-interface-theme/project/theme-optimization.md`  
生产说明：`designs/notora-interface-theme/project/README.md`

**目标：** 修复共享浅色 UI 的强调文字、焦点、错误/警告正文、输入边界与行号可读性，并保持原有布局、bg_* 纸面和 accent 填充。

**架构：** 五个新角色以 sRGB 源值保存在 ColorPalette，随 ThemeDefinition.palette 传递和转换，经 ApplicationTheme → SettingsTheme 到达控件。ThemeFile.palette 支持可选覆盖，旧 TOML 继承基主题缺省值；app 向 ui 提供纯数据，ui 不依赖 app 状态。

**技术栈：** Rust、wgpu、共享 textora-ui、TOML、现有主题与控件单元测试。

## 全局约束与已确认决策

- 本轮仅 P0。保留浅/深 bg_*、布局、accent 填充值、Markdown 与 mindmap 配色；深色输入边框改为 `#8a7865`，行号保持生产值。
- 五个角色是 P0 前置，不留到 P2：`on_accent`、`accent_text`、`danger_text`、`warning_text`、`focus_ring`。
- 浅/深源值依次为：on_accent `#2a1808` / `#2a1504`；accent_text `#9a4815` / `#ffc080`；danger_text `#b3261e` / `#f08a7f`；warning_text `#8a5a00` / `#e5ca9a`；focus_ring `#c2410c` / `#f59a4a`。
- 浅色 input_border 为 `#9f8d76`，editor.line_number 为 `#75695c`；深色分别为 `#8a7865` / `#a99b8e`。
- 保留 highlight / inactive_highlight：decorations.rs:183、app_renderer.rs:1092 仍用于搜索；同源 hover 字段不因相等删除。
- 每个子任务最多修改三个文件。新增字段跨层接线完成后执行编译；任何提交前编译必须通过。不提交半接线状态。
- Bug 先用消费者/绘制输出复现测试证明失败，再最小修复；测试目标是可读性和选用正确角色，避免只复制实现。
- 不新增 schema/meta、主题列表、P1 registry 或用户主题 warning。用户主题不因低对比自动拒绝；warning 仅 P2 候选。
- README 继续记录生产状态。P0 代码通过后另行同步预览 tokens，不把候选值写成生产值。

## 子任务 1：源角色、颜色转换与应用语义（3 文件）

文件：

- `crates/ui/src/theme/color.rs`：五个源角色与 gamma_correct 遍历。
- `crates/ui/src/theme/mod.rs`：内置定义、测试定义、浅色边框和行号。
- `crates/ui/src/theme/application.rs`：五个语义角色从 palette 直接映射。

- [x] 在现有 theme 测试内先复现浅色 on-accent/正文/焦点缺口，并保护 bg_*、accent、Markdown、mindmap 与深色保留项。
- [x] 为所有 ColorPalette 构造点提供源值；ThemeDefinition 继续复用 palette，不另设平行色板。
- [x] 将五个新角色纳入现有转换，alpha 不参与 gamma，禁止在控件中硬编码 hex。
- [x] ApplicationTheme 暴露同名角色，保留原 accent/danger/warning 及按钮派生公式。
- [x] 执行 `cargo test -p textora-ui theme:: -- --test-threads=1`。若结构字段尚未接完导致编译缺口，先完成子任务 2 再统一验证，不提前提交。

## 子任务 2：Settings 与 TOML 继承兼容（2 文件）

文件：`crates/ui/src/theme/settings.rs`、`crates/ui/src/theme_file.rs`。

消费子任务 1 的五个 ApplicationTheme 角色；产出 SettingsTheme 同名角色及 PaletteFile 的可选覆盖字段。

- [x] 先补旧 TOML 缺失五字段继承基主题、新 TOML 显式覆盖五字段的解析测试；覆盖非法颜色仍按原解析规则报错。
- [x] `PaletteFile` 使用 `Option<String>`，resolve/apply 只在提供值时覆盖；不加入对比度拒绝规则。
- [x] SettingsTheme 映射五角色，focus_ring 不再映射 application.accent。
- [x] 执行 `cargo test -p textora-ui theme_file -- --test-threads=1`、`cargo test -p textora-ui theme:: -- --test-threads=1`、`cargo check --workspace`。预期全部通过，旧主题与新覆盖均可加载。

## 子任务 3：按钮文字、输入焦点与旧断言（3 文件）

文件：`crates/ui/src/widgets/button/style.rs`、`crates/ui/src/widgets/text_box.rs`、`crates/ui/src/widgets/settings_view/widget.rs`。

- [x] 先补/调整针对浅色段选字底配对和真实 TextBox 聚焦绘制色的回归测试，验证原实现失败。
- [x] segmented 使用实心 accent + on_accent；菜单/ghost/category 文字使用 accent_text，混色仍保留 accent。
- [x] TextBox 使用 focus_ring，保持现有边框宽度、布局和非焦点行为。
- [x] 更新 SettingsView 中 selected_foreground 与 category 的旧断言，保留 selected_background 为原 accent 的断言。
- [x] 执行 `cargo test -p textora-ui widgets::button -- --test-threads=1`、`cargo test -p textora-ui widgets::text_box -- --test-threads=1`、`cargo test -p textora-ui widgets::settings_view -- --test-threads=1`，预期焦点与字底配对测试通过。

## 子任务 4：checkbox/switch 强调填充上的文字与图形（2 文件）

文件：`crates/ui/src/widgets/checkbox.rs`、`crates/ui/src/widgets/switch.rs`。

- [x] 先用绘制命令输出测试复现浅色 accent 填充上的 text_inverse 勾形/图形可辨识性缺口，覆盖已选与聚焦态。
- [x] checkbox 勾形、switch 已选文字/图形用 on_accent，保留原 accent 填充，未选文字继续使用原语义角色。
- [x] 保留焦点通过 SettingsTheme.focus_ring 的链路，区分实心填充、图形、普通边界和焦点边界。
- [x] 执行 `cargo test -p textora-ui widgets::checkbox -- --test-threads=1`、`cargo test -p textora-ui widgets::switch -- --test-threads=1`；预期绘制输出、选中和焦点检查通过。

## 子任务 5：错误/警告正文消费者（本组 2 文件）

文件：`crates/ui/src/widgets/search_bar.rs`、`crates/notora-app/src/new_workspace_dialog.rs`。

- [x] 先测试“无匹配结果”与工作区创建错误说明的正文角色；原实现直接用 danger，应能复现浅色缺口。
- [x] 错误说明改用 danger_text；SearchBar 活动文字若直接使用 accent，同步改 accent_text，保留混色/填充。
- [x] 用 `rg -n '\.(danger|warning)' crates/ui/src/widgets crates/notora-app/src crates/app/src` 审计其他正文消费者。危险按钮、标题栏关闭图标、警告指示条、mindmap 配色不盲目替换；若有额外正文，按每组最多三文件追加同类子任务并记录路径，没有现存警告正文不新增 UI。
- [x] 执行 `cargo test -p textora-ui widgets::search_bar -- --test-threads=1`、`cargo test -p notora-app new_workspace_dialog -- --test-threads=1`、`cargo check --workspace`。预期错误路径测试与编译通过。

## 子任务 6：活动图标消费者（本组 3 文件）

文件：`crates/ui/src/widgets/title_bar.rs`、`crates/ui/src/widgets/editor_header.rs`、`crates/ui/src/widgets/virtual_card_list/mod.rs`。`list.rs` / `sidebar/mod.rs` 的 `item_accent` 是填充条，保留 accent。

- [x] 先补活动图标/文字的绘制或样式回归断言，使用真实承载面检查必要图形 ≥3、文字 ≥4.5。
- [x] 以 accent_text 替代活动图标/正文的 accent；选中行/卡片保留专用字底配对，填充与状态混色源保持 accent。
- [x] 用 `rg -n '\.accent' crates/ui/src/widgets crates/notora-app/src crates/app/src` 核查剩余消费者。若发现额外活动图标，按每组最多三文件追加同类子任务，不能全局替换。
- [x] 执行 `cargo test -p textora-ui widgets::editor_header -- --test-threads=1`、`cargo test -p textora-ui widgets::title_bar -- --test-threads=1`、`cargo test -p textora-ui widgets::virtual_card_list -- --test-threads=1`，预期活动态通过且填充回归不变。

## 子任务 5b：加密对话框错误正文（2 文件）

文件：`crates/ui/src/widgets/encrypted_note_dialog.rs`、`crates/ui/src/widgets/encrypted_note_unlock.rs`。

- [x] 真实错误文案绘制先见红，再接入 danger_text；运行 `cargo test -p textora-ui --lib encrypted_note -- --test-threads=1`。

## 子任务 6b：主题选择器的界面状态（1 文件）

文件：`crates/ui/src/widgets/mindmap_style_panel.rs`。

- [x] 选择器 UI 焦点、勾号、警告文案分别接 focus_ring、on_accent、warning_text；内部节点/分支预览配色不变。运行 `cargo test -p textora-ui --lib mindmap_style_panel -- --test-threads=1`。

## 子任务 6c：自绘数字输入边界（1 文件）

文件：`crates/ui/src/widgets/table_picker.rs`。

- [x] 自绘数字输入边界从装饰性 border_subtle 接到 control_border；实际绘制回归先红 1.172:1，再转绿。运行 `cargo test -p textora-ui --lib numeric_input_boundaries_remain_visible -- --test-threads=1`。

## 子任务 6d：原生验收发现的占位符正文（1 文件）

文件：`crates/ui/src/widgets/text_box.rs`。

- [x] 原生查看浅色搜索占位符过淡，真实绘制测试先红 1.6051:1；把 input_fg 40% alpha 接到已有 text_secondary。浅深 × 普通/聚焦 × 实际背景组合均通过 `cargo test -p textora-ui --lib placeholder_text_remains_readable -- --test-threads=1`。

## 子任务 7：维护主题对比度与状态验收（至多 3 文件）

文件：优先扩展 `crates/ui/src/theme/mod.rs` 与 `crates/ui/src/widgets/button/style.rs` 内现有测试；需要独立测试模块时新增 `crates/ui/src/theme/contrast_tests.rs`（第三个文件，在 mod.rs 接线）。

- [x] 源值审计使用准确 sRGB 分段转换与亮度公式；运行时使用已转换线性值直接合成与计算，不再次解码。当前 powf(2.2) 与标准 sRGB 有差别，测试明确标注两种口径，不趁机替换全局 gamma。
- [x] 对内置/仓库维护主题覆盖 normal、hover、pressed、selected、focused、disabled，透明色合成到真实承载面，边界检查实际内外相邻背景。启用且全不透明时检查数值门槛；禁用/全局淡入淡出验证 alpha 派生和合成行为，沿用现有按钮 alpha/disabled 测试，不要求低 alpha 正文仍达 4.5。
- [x] 严格比较未舍入数值：正文/选中文字/行号 ≥4.5；必要图形、焦点及输入边界 ≥3；禁用态记录例外，普通按钮边界增强留 P1，装饰分隔线不冒充 WCAG 必须 ≥3。
- [x] 不将任意用户主题纳入强制拒绝测试，不把 notora 的纸面方向约束强加到所有主题。
- [x] 执行 `cargo test -p textora-ui -- --test-threads=1`、`cargo check --workspace`。预期所有维护主题与控件回归通过。

## 子任务 8：生产说明与预览 token 同步（3 文件）

文件：`designs/notora-interface-theme/project/README.md`、`designs/notora-interface-theme/project/tokens.json`、`designs/notora-interface-theme/project/tokens.css`。

- [x] 在代码验证通过后，把新增五角色和浅色边框/行号写入生产说明与 token；保留原 bg_*、accent、深色结构线与行号；深色输入边框同步为 #8a7865。
- [x] 按真实消费者同步语义，说明分段、勾选/开关、活动图标和错误正文的角色；不写入 P1 候选纸面与按钮目标。
- [x] JSON 解析检查：`python3 -m json.tool designs/notora-interface-theme/project/tokens.json > /dev/null`。
- [x] 逐项对照运行时角色与预览取色；若预览组件代码还直接引用旧角色，另立每组最多三文件的消费者同步子任务，不把无效 token 写入当作视觉同步完成。

## 子任务 9：最终验证与状态归档（不超过 2 文档文件）

文件：本计划、必要时 `designs/notora-interface-theme/project/theme-optimization.md` 的实施状态。

- [x] 执行 `cargo fmt --all -- --check` 与 `cargo check --workspace`。
- [x] 执行 `./scripts/verify.sh`，要求架构边界、fmt、clippy、notora 单线程测试及其余工作区测试全部通过；记录真实结果，失败先复现并分析根因。
- [x] 浅/深通过真实绘制/回读测试检查 Button、NavigationItem、NoteCard、SearchField、SegmentedControl、Dialog 及勾选/开关、活动图标、焦点、错误正文；原生 CUA 抽查设置相关关键场景，未遍历所有产品页面。
- [x] 用 `git diff --check` 检查格式；确认 P1 未注册/应用、未删除搜索高亮、未修改 Markdown/mindmap 配色。
- [x] 将实际完成步骤勾选并记录命令结果；未执行检查不能写成通过。只有验证完成才可标记 P0 完成。

## P1/P2 留档边界

P1 保留暖纸候选与 notora 专属主题方向，本轮不实施。候选按钮色与现公式不符：按候选底色现公式浅色约为 button-bg `#fbf9f4`、border `#ccc2b2`，并非目标 `#f7f2e8` / `#98866f`；后续先确定语义覆盖或公式。`#f6e3cc` 是实心色，不是 18% 水洗；选中分为行卡实心纸+专用字、菜单/ghost水洗+accent_text、分段实心accent+on_accent 三类。

P1 边框按实际相邻背景重算；`#9f8d76` 对候选 window/navigation/control-bg 约为 2.78/2.97/3.002，不是全局 ≥3。候选 L* 为 94.31/96.96/99.04（overlay 99.33），不声称相邻 3–6。纸面方向约束只面向 notora 候选；Markdown 语法色保留但必须随新背景回归。

未来 notora 主题入口需覆盖 `runtime.rs`、`runtime/frame_runtime.rs`、`render.rs` 三处解析点。用户主题 warning、schema/meta、主题列表及字段去重属于后续独立议题，不纳入此次 P0。

## 实测调整

- 深色 accent_text 初稿 #f2a659 在 category 选中态为 4.309:1，ghost 水洗叠浮层约 3.91:1；改为 #ffc080，保留现有填充，最不利 ghost 组合约 4.93:1。
- 基线首跑因旧 LSH 构建缓存引用另一临时目录失败，`cargo clean -p lsh` 后原有 21 个主题测试通过，未改 LSH 源码。

- 已选深色开关悬停时，旧的前景混色使 thumb/track 为 2.066:1；勾号与已选滑块稳定使用 on_accent，hover 仍由表面和边框反馈。
- TextBox 实际内外背景矩阵发现深色旧 control-border 对 window/overlay 为 2.7206/2.6007:1；改 #8a7865 后为 3.4248/3.2740:1，不修改 line-strong。
- 主题预览卡内缩焦点对 paper 浅画布仅 2.023:1；焦点改绘于卡片外侧的 UI 底面，布局与预览配色保持，补首排绘制裁剪余量。

## 验证记录（已完成）

- 源值与数据链：主题测试、TOML 可选覆盖/旧主题继承/非法色字段定位均已通过；精确 sRGB 审计与运行时 gamma 口径分别测试。
- 状态矩阵：五类按钮 × 浅深 × normal/hover/pressed/selected 全部达标；选中透明底先与实际承载面合成。禁用和全局透明度沿用按钮现有 alpha 测试，刻意半透明态不强制正文阈值。
- 实际绘制：TextBox 浅深普通/聚焦边框读取真实 fill；checkbox/switch 浅深悬停，表格数字输入，活动图标/卡片标签，加密/搜索/创建笔记库错误正文均有回归。
- 焦点范围：主题预览卡将绘制、裁剪、键盘可见性统一到 CardFocusGeometry；高度 300 时 Home/End 在 1×/2×下完整可见。高度 220 容不下整卡时顶部对齐并保留面板裁剪，不宣称几何上不可能的完整显示。
- 原生界面：使用独立 `/tmp/TextoraP0Review.app` 和 `/tmp/textora-p0-preview-config` 启动当前构建；通过 CUA 查看浅/深设置面板、分段选中、开关、输入边界与焦点。已关闭临时应用，原 `com.dan.notora` 仍运行。未遍历所有产品页面；其余组件由真实 DrawCmd/回读测试覆盖。
- 最后一次 `./scripts/verify.sh` 在全部源码修改后完整通过（退出码 0）：架构边界、fmt、clippy、notora 单线程测试、其余工作区测试及文档测试均通过；`cargo check --workspace` 最终版本通过。日志：`/tmp/textora-p0-verify-complete.log`、`/tmp/textora-p0-workspace-check-final.log`。
- 独立审查已关闭全部问题；现有暂存区补丁与操作前逐字节一致。未提交、未推送。

- 原生验收发现占位符正文仍使用 input_fg 的 40% alpha；回归复现浅色 1.6051:1 后统一使用已有 ink-muted，浅深普通/聚焦绘制测试通过。最后源码修改后再次完整运行 verify，而不沿用此前通过结果。

最终结论：P0 完成，所有独立审查项已关闭；P1 色板仍是候选，未注册或应用。工作区与现有暂存改动保留在 `codex/theme-readability-p0`，未提交、未推送。
