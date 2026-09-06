# WYSIWYG 段落编辑语义系统修复实施计划

> 执行方式：subagent-driven-development，独立模块任务由子代理实现，主代理负责公共入口集成与最终验收。用户已授权按审查报告实施，阶段间持续执行。每个子任务最多修改三个文件。

**Goal:** 关闭空行边界审查中的创建、删空、边界删除及标题起点语义缺口。
**Architecture:** 使用既有 EditableParagraphMap 统一空段语义；把段落清空和邻接空段删除做成共用操作，再接入 Enter、Backspace、Delete 和选区编辑。
**Tech Stack:** Rust，textora-markdown，ViewPlugin 编辑协议，真实 Shaper，应用事务历史。

## 全局约束

- 全程中文；使用精准命名、语义常量、提前返回、互斥状态 enum；不新增 unwrap。
- 不让 ui 依赖 app；不引入光标几何补丁，不重建已统一的空段布局。
- 先失败复现再修，同一问题两次修改失败后重新审查根因；不叠加防御性补丁。
- 保留当前工作区已验证的列表分隔/CRLF 修复；不撤销用户更改。
- 普通文字编辑快速路径保留；旧语义更新须对应本轮明确的不变量。
- 每个子任务最多三个文件，集成分拆；完成前完整 ./scripts/verify.sh。

## Task 1：方向无关的段落清空

文件：editable_paragraph_edit.rs；必要时独立同目录测试模块（最多两个文件）。
接口：新增 `pub(super) fn erase_range(source: &str, erased: Range<usize>) -> Option<EditAugmentation>`，原 `erase_last_grapheme` 变成计算 grapheme 范围并委托。主代理随后统一连接 Delete 和选区，实施者不修改 augmenter/view 公共入口。

- [x] 在模块内加入真实解析/替换测试：`a\n\nx\n\nb` 清空 x 必须变为 `a\n\n\nb`；涵盖 LF/CRLF、多字符整段选区、相邻额外空段、容器段落和 Unicode。非整段范围不错误归一化。
- [x] `cargo test -p textora-markdown --lib editable_paragraph_edit` 记录失败。
- [x] 用原始块树和 EditableParagraphMap 判定内容是否被清空、所属容器以及需撤除的隐藏分隔，生成单一原子替换；复用现有 Backspace 入口。
- [x] 运行模块测试并写自审报告，随后任务审查。

## Task 2：正文与标题的创建边界

文件：augmenter.rs、view_paragraph_semantics_tests.rs、view_empty_paragraph_tests.rs（挂载测试）。后续创建审查子任务独立修改 augmenter.rs、view_paragraph_semantics_tests.rs、view_heading_container_tests.rs，仍控制每个子任务最多三个文件。

- [x] 正式接入报告的失败矩阵：正文/H1/H2 × 文首、EOF、已有终止换行、块间、空格/Tab 分隔 × LF/CRLF；每次 Enter 增加一段。
- [x] 统一段尾创建时的空段计数查询；段首只前插一段；标题用显式起点/中部/末尾分类，Setext 起点保留原标题。
- [x] 运行 Enter 定向测试；只有旧预期与现行规范明确冲突时更新，并同时保留语义/几何断言。
- [x] 审查公共输入语义，保留列表/引用退出与代码/表格专用行为。

## Task 3：邻接空段的方向对称删除

文件：editable_paragraph_navigation.rs、必要时 boundary helper/tests（最多两个新增文件）。
接口：`pub(super) fn backspace_boundary(source: &str, current_byte: usize) -> Option<EditAugmentation>`；`pub(super) fn delete_forward(source: &str, current_byte: usize) -> Option<EditAugmentation>`。主代理负责在旧块合并/marker保护之前连接。

- [x] 失败测试：`a\n\n\n\nb` 从 b 首 Backspace、从 a 尾 Delete 只少一个可编辑空段；标题/规则/引用/列表边界不得越过空段合并。
- [x] 用 EditableParagraphMap 找同一容器最近空段，删除整行及完整 LF/CRLF，保留有效隐藏分隔。正文/标题前导空段同样逐个删除；代码体空行不走普通段落分支。
- [x] 处理空段本身的前向删除以及 EOF 的隐藏分隔归一化，避免多按一次才消失或误删前后块。
- [x] 定向测试、自审、任务审查。

## Task 4：公共入口和选区事务集成

文件：augmenter.rs、view.rs、view_paragraph_semantics_tests.rs。

- [x] 先写 EditPolicy 实际入口测试，验证 Backspace/Delete/选区清空相同内容后源码与布局一致。
- [x] 为 Delete 计算完整 grapheme 删除范围并调用 Task 1；为 DeleteBackward/DeleteForward 选区调用同一个 erase_range；仅安全支持的范围消费增强，其他默认路径保留。
- [x] 在真实块合并前连接 Task 3；更新边界旧测试使其断言“有空段先删一个，无空段按旧块规则”。
- [x] 完整 Markdown 库测试；检查快速路径和公共协议编译。

## Task 5：应用历史、文档、综合验收

先修改 app_tests.rs（最多一个文件），随后文档子任务（本计划、规格、旧行为规范最多三个文件）。

- [x] 应用层真实事务测试覆盖空段 Enter/Backspace、Delete/选区删除、标题前插空段的 Undo/Redo 和光标恢复。
- [x] 真实 View 矩阵验证首次输入/删除几何、1×/2× DPI、点击/IME、冷启动与重排。
- [x] 更新旧 Enter/Backspace 行为规范的文末、整串换行删除、InsertText 修剪和标题起点描述；审查报告保留基线，追加本轮完成记录。
- [x] 独立整体审查并解决有证据的 P1/P2；执行 `./scripts/verify.sh`，记录结果。

## 进度

- 基线：HEAD 23f4bba + 当前工作区列表回车/CRLF修复；1219 Markdown测试通过，完整verify已通过。
- Task 1：统一段落范围清空及多行链接完整语法删除已完成，公共 Backspace/Delete/选区入口已接入。
- Task 2：文首、EOF、空格/Tab 分隔、标题起点创建已接入；追加审查的可见样式起点、已有空首列表项、紧贴前块的隐藏分隔三项均已修复，新增创建矩阵 4 项和段落集成矩阵 17 项通过。
- 独立布局子任务：Setext 只使用实际源码的标题标记类型，修正假 ATX 标记引起的光标缺失（layout/block.rs，独立于三文件创建任务）。
- Task 3：最近空段导航、跨容器中性分隔、样式边界和列表首行 marker 保留已接入；原有代码/列表护栏仍保留。
- Task 4：三种删空方向的源码矩阵通过；真实布局、1×/2× DPI、点击命中验证通过。第一次全库集成暴露的旧 EOF 预期已按新规范更新；第二次暴露的两个列表保护回归已定位为 map 首空行候选误识别，已以真实 marker-only 源码校验修复，原保护预期保持并通过。
- Task 5：应用事务的 Enter、Backspace、Delete、选区删除与用户列表原例，Undo/Redo 恢复源码和光标均通过；包含从真实 CRLF 文件加载的 Unicode grapheme 删除，保留精确换行编码。历史原有契约会折叠选区，未扩大为修改历史模型。
- 独立最终审查已完成：创建和非创建部分均未留下有证据的 P1/P2；当前完整 verify 已通过，结果如下。


## 最终验收

`./scripts/verify.sh` 完整重跑，退出码 0，输出 `All checks passed! Baseline is trusted.`。

- 架构边界检查、`cargo fmt --all -- --check`、全工作区 Clippy（warnings as errors）通过。
- `textora-markdown`：1256 通过，0 失败。
- `textora-app`：916 通过，0 失败，2 项既有忽略测试保持不变。
- `notora-app` 单元测试：344 通过；启动、恢复、保存、搜索等集成测试通过。
- 同步 mock：27 通过；其余工作区单元测试、集成测试与 doctests 通过。
- 第一次全量运行发现应用旧测试要求吞掉已有 EOF 空段，按本规范改为保留尾空段，并增加新正文光标位置断言；随后完整重跑通过。
- 非创建逻辑与创建逻辑分别完成独立复审，未留下有证据的 P1/P2。

验收日志：`/tmp/textora-systemic-repair/verify-final.log`。提交与远端发布状态以 Git 历史记录为准。
