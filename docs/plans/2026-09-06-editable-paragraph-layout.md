# 可编辑空段落统一布局实施计划

> 按 subagent-driven-development 与 TDD 分阶段执行；用户已授权开始实现，无额外设计审批。每个子任务最多修改三个文件。

**Goal:** 消除空光标、输入后文字与退格后空段落的位置、字号、间距不一致。

**Architecture:** 编辑专用布局树注入零宽普通 Paragraph，沿用现有块布局；所有交互读取真实行几何，删除旧的高度补偿和反推。

**Tech Stack:** Rust、textora-markdown、现有 Shaper 与 ViewPlugin 测试。

## 全局约束

- 不改变Markdown存储格式与纯预览/小说行为。
- 不新增元素专属光标偏移，不让ui依赖app。
- 不使用unwrap；先复现再修改；同一问题两次修改失败后重新审查设计。
- 提交前编译通过，重大修改完成运行 ./scripts/verify.sh。

## 子任务与顺序

1. 回归矩阵（view.rs、view_empty_paragraph_tests.rs）：真实增强源码、重排、检查空态/输入/删除几何；运行 cargo test -p textora-markdown --lib empty_paragraph_tests，确认原实现失败。
2. Backspace（augmenter.rs，子代理）：LF/CRLF及多空行逆操作，修正文中落点；独立RED/GREEN与审查。
3. 编辑树（builder.rs、editable_paragraphs.rs，子代理）：实现 build_for_editing 与 is_editable_empty_paragraph，测试范围归属和零宽节点；不接入生产调用直到集成阶段。
4. 撤除额外补偿（layout/types.rs、layout/block.rs、layout/context.rs）：删除顶层和嵌套reserve，完全由普通段落占位；调整受影响的布局测试。
5. 统一交互几何（view.rs、layout/types.rs、layout/source_line_map.rs）：编辑入口使用新树；从真实行建立零宽投影；删除source-only几何和view邻接反推；保留隐藏分隔源码映射。
6. 连续输入与边缘语义（augmenter.rs、view_empty_paragraph_tests.rs）：必要时修正源码边界规范化以保持剩余空段落数量；测试文首、文末、空文档和空行串中的任意落点。
7. 复用与回归（layout/reconcile.rs、layout/types.rs、view_empty_paragraph_tests.rs）：针对新零宽节点校验增量布局=冷启动、IME、点击和导航。只修改测试揭示的实际问题。
8. 收尾审查（文档）：独立代码审查、cargo fmt检查、Markdown全库和 ./scripts/verify.sh；记录通过数和限制。

独立审查后的第6阶段细分：

- 6a（builder.rs、editable_paragraphs.rs）：抽共享 EditableParagraphMap 与无样式 build_structure，让布局和编辑共用归属。
- 6b（augmenter.rs、editable_paragraph_edit.rs、view_empty_paragraph_tests.rs）：替换字符插入及最后一个字删除的换行扫描，补容器和空格空行往返矩阵。
- 6c（augmenter.rs、editable_paragraph_navigation.rs）：空段落 Enter/Backspace 共用同一归属表；保留空列表项和引用退出命令。
- 6d（layout/block.rs）：修复引用空段落展开标记时的源码范围，禁止前缀投影越过物理行边界。

## 进度

- 回归矩阵原实现：3个测试因几何断言失败，已确认RED。
- 已完成编辑树、共享空段落归属表、输入/删除/导航、IME及源码投影集成。
- 已删除旧顶层/嵌套空行高度补偿、去重状态及view邻接字号/位置反推。
- 独立审查发现的列表脱离容器、引用合并后段、EOF吞槽、全空文档退格跳槽、带空格空行语义不一致均已复现并关闭；最终审查无剩余P1/P2。

## 最终验证（2026-09-06）

- `cargo check --workspace`：通过。
- `./scripts/verify.sh`：完整通过，包括架构边界、`cargo fmt --all -- --check`、工作区所有目标Clippy（警告作为错误）、notora-app串行测试及其余工作区测试/文档测试。
- Markdown最终单元测试：1217通过，0失败。元素输入/删除往返、LF/CRLF、1×/2×DPI、嵌套引用/列表、空格输入、文首/文末、连续Enter/Backspace、IME/点击、增量与全量布局一致均覆盖。
- 首次沙箱内全验证在同步模块本机mock端口绑定处遭遇PermissionDenied；经执行权限审批后重新完整运行，全部通过，未跳过失败测试。
- 当前验证覆盖真实ViewPlugin排版/绘制产物和编辑协议，未进行原生窗口键盘及系统输入法手工操作。

## 用户可见行为

空段落具有实际行高、段间距和所属容器的正文样式。容器外空段不再继承前标题字号或列表缩进；输入文字不会改变该段原有的纵向位置或吞掉其他空段。Backspace逐个撤回可编辑空段，只有单个空容器时保留既有退出命令。Markdown存储格式、纯预览和小说模式保持原有入口。
