# Textora 与 Notora 新人接手指南

先看 [README 的启动命令和 crate 职责](../README.md)，再用这份指南追一次真实操作。不必先读历史计划，也不要假设两个产品走同一条事件链。

## 先认清状态归属

| 想知道什么 | 去哪里看 |
| --- | --- |
| 谁收到系统事件、决定当前操作？ | Textora 从 [`main.rs`](../crates/app/src/main.rs) 进入 [`app_lifecycle.rs`](../crates/app/src/app_lifecycle.rs)；Notora 从 [`main.rs`](../crates/notora-app/src/main.rs) 经 [`app.rs`](../crates/notora-app/src/app.rs) 进入 [`runtime.rs`](../crates/notora-app/src/runtime.rs)。产品层处理焦点、菜单、对话框及各自的业务策略。 |
| 正文、选区、撤销和脏状态在哪？ | [`DocumentModel`](../crates/appkit-core/src/document/model.rs) 持有 [`TextBuffer`](../crates/core/src/buffer/text_buffer.rs)，并维护光标、行索引、内容修订号和保存状态。缓冲区负责正文与撤销历史。 |
| 当前标签的视图和几何在哪？ | 共享宿主 [`EditorRuntime`](../crates/appkit-shell/src/editor_runtime/mod.rs) 按 `TabId` 组织编辑器工作区；[`TabRuntime`](../crates/appkit-shell/src/tab_runtime.rs) 持有视图插件和展示状态；[`TabSession`](../crates/appkit-shell/src/tab_session.rs) 借用模型和运行态。`DocumentPresentation` 不是正文的权威来源。 |
| 笔记库和编辑器工作区是同一个东西吗？ | 不是。Notora 域的 [`Workspace`](../crates/notora-core/src/workspace.rs) 指笔记库根及其元数据；壳层的 [`Workspace`](../crates/appkit-shell/src/workspace.rs) 管打开的编辑器标签。笔记标识和 `TabId` 不能互换。 |

定位改动时，遵守 [`AGENTS.md`](../AGENTS.md) 的依赖规则：产品层把状态映射为纯数据输入，`ui` 不读取产品状态；共享壳层通过 [`ViewPlugin`](../crates/ui/src/plugin.rs) 使用视图能力，不直接依赖 Markdown 实现。

下面的走查针对流式 Markdown。`.mmap.md` 由 [`textora-mindmap`](../crates/mindmap/src/lib.rs) 提供画布视图，共用编辑器宿主，但不走流式 Markdown 排版。思维导图样式面板在 [共享壳层](../crates/appkit-shell/src/mindmap_style_panel.rs)，Notora 的加密笔记对话框在 [Notora 产品层](../crates/notora-app/src/encrypted_note_dialog.rs)。

## 用一份临时文档走查

在仓库根目录创建一个样例，先任选一个产品打开；退出后再用另一个产品重复，不要同时编辑同一个文件。

```bash
sample_dir=$(mktemp -d)
sample_path="$sample_dir/onboarding.md"
cat > "$sample_path" <<'EOF'
# 接手样例

这一段用来观察自动换行：**粗体中文前半段粗体中文后半段，点击这里再输入一个字**，然后撤销并保存。
EOF

# 二选一；Notora 的这个入口按外部文件打开，不导入笔记库。
cargo run -p textora-app --bin textora -- "$sample_path"
# cargo run -p notora-app --bin notora -- "$sample_path"
```

切到 Markdown 所见即所得编辑视图（不是源码或只读预览），缩窄编辑区直到段落换行。点击后半段的粗体中文，插入一个字，撤销一次，再重新输入并保存。最后切到源码或重开文件，确认保存的是 Markdown 源码，而不是屏幕上折行后的文字；视觉换行不会新增源码换行。下面按「点击 → 编辑 → 保存」的顺序追踪这次操作。

## 1. 点击映射：从屏幕像素回到源码字节

1. **产品先路由事件。** Textora 的自绘视图分支在 [`dispatch/mouse.rs`](../crates/app/src/dispatch/mouse.rs)，从 `dispatch_editor_mouse_input` 找到 `query_plugin_edit_hit_target`。Notora 从 [`NotoraRuntime::route_pointer_event`](../crates/notora-app/src/runtime.rs) 进入共享宿主的 [`handle_pointer_event` / `place_plugin_pointer_selection`](../crates/appkit-shell/src/editor_runtime/editor_pointer.rs)。
2. **视图做坐标转换。** 两路都能通过 `TabSession` 查询 `PluginQuery::HitTestEditTarget`。[`MarkdownEditorView::query`](../crates/markdown/src/view/editor_view.rs) 对正文返回 `EditHitTarget::TextCaret`。[`hit_test_anchor`](../crates/markdown/src/view/engine_navigation.rs) 用渲染原点和滚动量算出文档坐标，找到排版后的视觉行，再用横坐标求字素边界。
3. **投影找回源码，宿主放置光标。** 在 [`anchor_from_flat_line_and_visual_grapheme`](../crates/markdown/src/view/engine_navigation.rs) 跟进 [`SourceProjectionIndex::source_anchor_at`](../crates/markdown/src/projection.rs)，可看到视觉行与字素索引如何还原为 `SourceAnchor`。Textora 的分支还查询视觉光标来保留亲和性，再更新模型光标和选区；共享宿主的正文命中分支按返回的源码字节更新模型并向插件发送 `SetCursorByte`。

**排查时区分三种量：** 像素是几何位置，视觉行和字素是本次排版里的位置，源码偏移是 UTF-8 字节索引。粗体标记可能不显示，中文又占多个字节，不能靠数屏幕字符算偏移。投影同时关联源码代数和布局修订号；换行边界的同一源码字节还可能对应两个视觉落点，由光标亲和性区分。纯文本视图不走这条 Markdown 投影链。

**纯文本视图：** Textora 的 [`hit_test_with_sub_line_offset`](../crates/app/src/mouse.rs) 和共享宿主的 [`hit_test_text_byte`](../crates/appkit-shell/src/editor_runtime/editor_pointer.rs) 在 [`cursor_motion.rs`](../crates/appkit-shell/src/cursor_motion.rs) 共用两步查找：`hit_test_visual_row` 用窗口 Y 坐标和折行滚动量找可视行，`hit_test_cluster_end` 用窗口 X 坐标找字符簇边界。`hit_test_row_grapheme` 和 `hit_test_row_byte` 保留各自的零宽簇处理和浮点中点算法；Textora 仍将行内字素偏移连同可视行返回，以区分折行边界的两个落点，共享宿主仍返回行内字节偏移。

**纯文本选区：** 三击选的是源码行，不是屏幕上的折行；两端都在 [`DocumentModel::source_line_selection_range`](../crates/appkit-core/src/document/model.rs) 取含换行符的字节范围。按住 Shift 单击时，两端也都用 `ensure_selection_active` 保留已有锚点，或以原光标为锚点。按词或行拖选时则分开读：Textora 的 [`handle_cursor_moved`](../crates/app/src/mouse.rs) 按第一次移动的方向固定锚点，共享宿主的 [`place_grouped_drag_selection`](../crates/appkit-shell/src/editor_runtime/editor_pointer.rs) 保存初始范围，拖回时恢复整词或整行。

可从 [`view_tests.rs`](../crates/markdown/src/view/view_tests.rs) 的粗体中文命中和后续视觉折行命中用例开始：

```bash
cargo test -p textora-markdown --lib hit_test_byte_roundtrip_inside_cjk_bold_span
cargo test -p textora-markdown --lib hit_test_byte_on_later_plain_soft_wrap_uses_segment_source_range
cargo test -p textora-appkit-shell --lib horizontal_hit_
```

## 2. 编辑提交：从编辑意图到缓冲区变更

1. **先找到产品入口。** Textora 从 [`dispatch_edit_command`](../crates/app/src/dispatch/editor.rs) 将适用的输入转为 `EditIntent`，进入 `dispatch_transactional_edit`；并非每条 `EditCommand` 都走事务，其他命令仍有导航和传统执行器等分支。Notora 的 [`commit_editor_text` / `handle_editor_key_input`](../crates/notora-app/src/runtime.rs) 则把编辑区输入交给 [`EditorRuntime::commit_text` / `handle_key_input`](../crates/appkit-shell/src/editor_runtime/mod.rs)。输入法的预编辑不是正文提交。
2. **把意图变成计划。** [`EditRequest`](../crates/ui/src/plugin.rs) 带上当前源码代数、光标和选区；[`MarkdownEditorView::plan_edit`](../crates/markdown/src/view/editor_view.rs) 为 Markdown 结构决定编辑策略。插件返回 `UseDefault` 时由调用方补默认计划；移动光标、改变选区或消费按键不等于修改正文。
3. **沿实际执行器找写入点。** Textora 调用 [`edit_transaction::execute_edit_plan`](../crates/app/src/edit_transaction.rs)；Notora 调用共享壳层 [`ModelSession::edit_active_document`](../crates/appkit-shell/src/editor_runtime/model_session.rs) 中的 `apply_edit_plan`。两套实现分别验证代数与替换范围，按撤销历史规则写入 `DocumentModel` 持有的缓冲区，再更新展示和插件源码。不要假定在一边改默认行为，另一边就会变化。
4. **分清「已编辑」和「已落盘」。** 正文变化后再看 `ContentChanged`、`DirtyChanged` 等通知。Notora 在 [`handle_editor_notification`](../crates/notora-app/src/runtime.rs) 用内容修订号调度符合条件的笔记自动保存；编辑提交本身并不代表磁盘已写入。

对应测试分别覆盖 Textora 编辑后撤销，以及共享宿主连续输入的撤销合并：

```bash
cargo test -p textora-app --lib editing_boundary_audit_undo_redo_restores_exact_source_and_cursor
cargo test -p textora-appkit-shell --lib typing_without_navigation_coalesces_into_one_undo_entry
```

## 3. 保存完成：从异步快照到清除脏状态

1. **产品决定何时保存、保存什么。** Textora 从 [`save_active_entry`](../crates/app/src/dispatch/commands.rs) 进入 [`submit_editor_save`](../crates/app/src/app_tab.rs)，必要时先选择另存路径。Notora 的 [`DocumentRuntime::save_manually`](../crates/notora-app/src/runtime/document_runtime.rs) 将工作区笔记交给自动保存调度器立即处理；已有外部文件直接提交保存，未命名外部文件先选路径。本指南的临时文件走外部文件分支；要观察自动保存，应在 Notora 的测试笔记库新建一篇笔记。
2. **共享宿主准备快照并交给后台。** [`EditorRuntime::prepare_save` / `prepare_save_as`](../crates/appkit-shell/src/editor_runtime/mod.rs) 准备带 `TabId`、内容修订号、目标路径、预期磁盘版本和序列化内容的 `PreparedDocumentSave`。[`document_save.rs`](../crates/appkit-shell/src/editor_runtime/document_save.rs) 的后台路径调用 [`save_serialized_if_unchanged`](../crates/appkit-core/src/file_safety.rs)，发现外部修改则返回冲突；加密笔记可在落盘前转换保存载荷。
3. **产品收结果，模型决定是否变干净。** Textora 在 [`drain_save_results`](../crates/app/src/app_lifecycle.rs) 收取结果；Notora 在 [`DocumentRuntime::drain_save_completions`](../crates/notora-app/src/runtime/document_runtime.rs) 收取结果。两边都调用 `EditorRuntime::apply_save_completion`，成功结果按 `TabId` 找回文档。仍存在的文档由 [`DocumentModel::apply_save_completion`](../crates/appkit-core/src/document/model.rs) 更新磁盘版本，但**只有保存的内容修订号仍是当前修订号，才会清除脏状态**。因此「快照写成功」不等于「刚刚的新编辑也保存了」。
4. **最后才处理产品后续动作。** 例如 Textora 的保存后关闭、Notora 的冲突重试或待保存再移动的笔记。Notora 在 [`apply_document_outcome`](../crates/notora-app/src/runtime.rs) 依次处理通知、产品动作和[文档命令](../crates/notora-app/src/runtime/document_commands.rs)，再应用壳层 effect；例如未命名外部文件另存为，要等[路径规范化完成](../crates/notora-app/src/runtime/document_completions.rs)并更新会话后，才触发待处理的关闭。排查时同时核对 `TabId`、保存的内容修订号和当前脏状态，而不是只看成功通知。

可直接验证旧快照完成后保留新编辑，以及已关闭标签的成功结果不再更新文档：

```bash
cargo test -p textora-appkit-core --lib stale_save_completion_keeps_newer_edits_dirty
cargo test -p textora-appkit-shell --lib late_save_completion_after_close_is_ignored
```

## 改动前后怎么验证

- **先跑离改动最近的测试。** 点击错误先区分几何命中和源码投影；输入或撤销错误先确定产品执行器；保存错误先区分磁盘冲突、旧修订完成和产品后续动作。
- **跨层改动再跑** [`./scripts/verify.sh`](../scripts/verify.sh)。[README](../README.md#验证) 说明它默认覆盖哪些检查，以及如何单独跑 `textora-sync`；常规验证不是桌面交互测试。
- **编辑行为涉及原生窗口时，补一次手动走查。** [`scripts/verify-editing.sh`](../scripts/verify-editing.sh) 提供专项用例和审计产物；脚本不会替你实际操作窗口。用同一份临时 Markdown 分别检查两个产品的点击、输入、撤销和重开结果。
