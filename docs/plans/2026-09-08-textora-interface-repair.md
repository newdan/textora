# Textora 界面修复计划

**目标：** 完整处理本次界面评估发现的布局、可读性、一致性问题，以当前 Textora 构建的实际窗口和测试为准。

**设计：** 保留原生编辑器布局、文档模式和橙色强调色。侧栏操作合并为一行，窄侧栏的打开按钮收为图标；外壳使用统一的字号、圆角和主题语义。标题、工具栏必须在窄窗口中保持按钮可见且文本不重叠。正文排版沿用当前引擎，避免无关重构。

**架构约束：** UI 只消费纯数据，不访问 app 状态。尺寸只缩放一次 DPI。每个子任务控制在三个文件以内；发现缺陷先写复现测试。共享控件的变化同时检查 Notora 的已有测试，产品名称修复仅作用于 Textora。

## 任务 1：侧栏布局

文件：`crates/ui/src/widgets/sidebar/state.rs`、`mod.rs`、`widget_tests.rs`。

- [x] 将新建（保留下拉）和打开合并为同一行；打开按钮在窄侧栏维持 32 logical px 命中区域，宽侧栏使用 72 logical px 并显示文字；文件列表上移。
- [x] 标题区汉堡按钮的绘制和命中统一为 32 logical px，避免扩大后侵占系统窗口按钮。
- [x] 侧栏选中项使用 navigation_selected_text；普通文件名使用主文字，分组标题使用次文字。
- [x] 测试 160/220/400 宽度、1x/2x DPI 下操作不交叠、按钮可点击、文件列表与设置按钮不重叠。

## 任务 2：标题与标签栏（顺序拆分）

2a 文件：`crates/ui/src/widgets/title_bar.rs`。

- [x] 先写长中文文件名、长路径、窄区域的绘制裁剪回归测试。
- [x] 标题文本限制在操作按钮之前，长文件名省略，路径空间不足时隐藏或省略；tooltip 保留完整信息。
- [x] 标题使用主文字 13 logical px，路径次文字 11 logical px，不额外降低透明度；垂直居中。
- [x] 操作按钮使用共享圆角与 hover token，几何始终非负；覆盖 light/dark、1x/2x。

2b 文件：`crates/ui/src/widgets/tab_bar/state.rs`、`layout.rs`、`widget.rs`。

- [x] 标签测量、绘制和 tooltip 使用相同的 13 logical px 字号。
- [x] 活动标签与 hover 使用语义主题，圆角统一共享控件 token；保留固定、关闭、滚动和溢出行为。

## 任务 3：Textora 搜索替换栏

调用链核实：`EditorToolbarWidget` 仅由 `notora-app` 使用，Textora 不显示这一独立工具栏。原任务中的分组/溢出改动不适用于 Textora。Textora 顶部操作由任务 2 覆盖，本任务修复实际使用的搜索替换栏。

文件：`crates/ui/src/widgets/search_bar.rs`，必要时新建 `search_bar/layout.rs`（每次子任务最多三个文件）。

- [x] 先测试复现 find-only 布局错误预留替换按钮宽度，以及窄窗口输入框与按钮交叠问题。
- [x] 布局、绘制和命中共用几何；按钮区域在 set_rect 确定，绘制前也可点击。
- [x] 搜索框背景使用 input_bg，边框用 stroke，避免第二次填充覆盖背景。
- [x] 长查询裁剪在输入区内，计数过长时收缩/隐藏辅助信息；窄窗口保留关闭、查找输入与模式入口，任何按钮不跨越边界。
- [x] 运行现有搜索/替换/输入法/鼠标回归测试并检查实际 GUI。

## 任务 3b：Markdown 正文默认样式

实际示例截图确认引用与表格背景过亮；代码发现半透明白色/强调色直接在线性颜色空间混合、H4大于H3、代码块背景主题字段被忽略。

文件：`crates/markdown/src/style.rs`、`crates/ui/src/theme/mod.rs`。

- [x] 标题字号随语义深度递减，引用与表格使用已解析的低对比不透明底色。
- [x] 代码块使用 `markdown.code_block_bg`，保持用户主题语义。
- [x] 回归测试先失败后通过，再运行 Markdown 全部测试并以截图确认。

## 任务 4：主题与状态信息

文件：`crates/ui/src/theme/mod.rs`、`crates/ui/src/widgets/status_bar.rs`、`crates/ui/src/constants.rs`。

- [x] 统一暗色输入框和侧栏的冷灰底色，保留橙色强调色；亮色主题对应保持清晰层级。
- [x] 状态栏使用 11 logical px 的辅助文字与语义色，将坐标/选区缩写改成清晰中文；冲突提示保留。
- [x] 修复选区统计缓存的失效判断时先用同终点、同字节数但字符数改变的用例复现。
- [x] 运行 UI 全部测试和主题相关绘制检查。

## 任务 5：Textora 名称与完整验收

5a 文件：`crates/app/src/app_window.rs`、`app_tab.rs`、`dispatch/tabs.rs`。

- [x] 初始窗口、打开文件和弹出窗口标题统一使用 textora，保留历史配置目录兼容。

5b 文件：`crates/app/src/native_menu.rs`、`main.rs`、`cli.rs`。

- [x] 本机菜单、命令行产品名统一为 textora。

5c 验收与归档：

- [x] `cargo build -p textora-app`。
- [x] `./scripts/verify.sh`（架构、fmt、Clippy、Notora 串行测试、剩余工作区测试）。
- [x] 真实运行检查：Markdown 编辑/预览、纯文本、明暗主题、侧栏/标签模式、设置与搜索、窄窗口。
- [x] 保存修改前后截图与验证结果，最终代码审查，修复发现的问题。

## 已确认基线

- 初始 git 工作区干净，HEAD `550785d`。
- `cargo build -p textora-app` 成功。
- 当前代码已构建为 `target/Textora UI Review.app`，通过 Computer Use 查看了真实空文档和阅读模式窗口。
- 截图确认窗口名称残留 edit+、侧栏顶部纵向操作占位、暗色外壳色调不一致。

## 联调追加任务：目录与搜索焦点

- [x] 修复展开目录后正文仍使用旧视口原点、左侧被目录遮挡的问题；先复现再修复并验证真实窗口。
- [x] 修复原生编辑菜单绕过搜索焦点的问题：查找/替换字段获得焦点时，全选、复制、剪切、粘贴与快捷键走同一控件路径；撤销/重做不再落入正文。
- [x] 菜单焦点回归先失败后通过，覆盖两个搜索字段的全选与中文提交、正文不变、文档焦点正常下传。
- [x] 最终构建验证目录布局和中文粘贴，完成增量审查。

## 已完成验证记录

- `cargo test -p textora-ui`：1056 个单元测试、14 个集成测试通过，1 个文档示例忽略。
- `cargo test -p textora-markdown`：1284 个单元测试与12个集成测试通过。
- `cargo test -p textora-app native_`：9 个相关测试通过。
- 全局审查与目录、菜单焦点、统一事件路由增量补审均通过。
- 实际窗口已核验深浅主题、侧栏与标签页模式、设置入口、状态栏、长查询和替换栏窄窗口裁剪。
- 侧栏追加拆分涉及 `types.rs`、`persistent.rs` 与 `appkit-shell/ui_shell.rs`：统一响应式可见性计算，移除改变布局宽度的重复动画，保证缩窄/恢复窗口当帧同步 Dock。

## 最终验收（2026-09-08）

- `./scripts/verify.sh` 完整通过：架构检查、格式检查、Clippy（警告视为错误）、Notora 串行测试及其余工作区测试；累计4800项通过、5项按既有设置忽略。
- 同步测试需要绑定本机回环端口；沙盒内被拒绝后，经自动审批允许在沙盒外运行现有验证脚本，最终完整通过。首次Notora加密测试曾因3秒时限超时，独立复跑与后续完整复跑均通过。
- `cargo build -p textora-app` 最终成功；实际验收的是本仓库构建的 `target/Textora Validation.app`，与 Notora 区分。
- 最终窗口确认目录展开后正文整体右移并重新换行，关闭恢复；中文原生粘贴进入搜索字段，正文不变。Computer Use 的剪贴板等待有超时报文，但后续截图清楚确认输入结果。
- 最后回归同时修正菜单动作必须经过统一 `dispatch()` 的边界约束，以及侧栏集成测试过时的行坐标。
- 截图保存在 `/tmp/textora-ui-review/`：`final-textora.png`、`after-toc-fixed.png`、`final-search-validation.png`、`after-native-replace-paste.png`、`after-light.png`、`after-tabs-status.png`。
- 修改保留在当前工作区，未提交、未推送。

## main 整合补记

发布前保留远端新增的共享 `ButtonStyle`、斜体渲染裁剪修复，以及打开按钮随侧栏宽度显示文字/图标的适配。新建与打开共用底色、前景色、边框和圆角；布局回归覆盖160/220/400宽度与1x/2x缩放，颜色回归同时核对按钮标签和图标。
