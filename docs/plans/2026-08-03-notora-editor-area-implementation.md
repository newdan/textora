# Notora 编辑区执行计划

日期：2026-08-03

状态：未加密编辑区已完成自动化收尾；真实加密路径由 2026-08-20 加密笔记实施方案接续，人工 UI 验收尚未执行

范围说明：原 Task 7.3 已被 2026-08-20 加密笔记实施方案替代。本计划保留既有未加密编辑区记录，不再重复规定加密创建、解锁、保存和生命周期实现。

关联规格：[`2026-08-03-notora-editor-area-design.md`](../specs/2026-08-03-notora-editor-area-design.md)

## 1. 目标

把 Notora 右侧编辑区从单一正文画布升级为完整的文档编辑界面：

- 标题栏直接编辑正文标题真源；
- 展示创建时间、修改时间、保存状态、星标、只读加密状态和删除入口；
- 在当前活动工作区内选择笔记目录；
- 通过头部属性区维护正式层级标签；
- 提供按文档类型变化的编辑器菜单；
- 在普通笔记、外部文件、回收站和空状态之间保持明确边界；
- 保持 EditorRuntime、自动保存、冲突处理、索引和响应式布局正确。

## 2. 当前基线

现有实现已经具备：

- 固定三栏与紧凑窗口模式；
- 工作区内创建、重命名和移动文件；
- 自动保存、保存冲突和 dirty snapshot；
- catalog 星标、标签表、笔记标签关联和 FTS 标签字段；
- 从正文提取 hashtag 并覆盖 `note_tags` 的扫描链路；
- TXT、Markdown 和 Mindmap 编辑器运行时；
- 位于中栏标题区的重命名、移动、星标和回收站按钮。

当前缺口：

- catalog 没有创建时间和加密属性；
- 仓库没有真实的笔记加密编解码、密钥管理或加密保存实现；
- EditorRuntime 没有面向产品的“读取文本快照并执行范围替换”公共契约；
- 正文扫描仍会用 hashtag 覆盖正式标签；
- 没有头部标签编辑、目录选择、创建面板和编辑器菜单组件；
- 当前格式化编辑协议只支持插入、删除、缩进和 Mindmap 结构操作；
- 编辑区布局只有一个 `editor_rect`，没有文档头部、菜单和正文子区域。

当前工作区另有未提交的 EditorRuntime、Notora render/action/state 和 UI 变更。执行本计划前必须先确认这些改动的归属并形成可编译基线；不得覆盖或回退它们。

## 3. 全局约束

- 产品名保持 `textora`，Markdown 包名保持 `textora-markdown`，笔记产品名保持 `notora`。
- `ui` 只定义纯数据输入和通用 widget，不得依赖 `NotoraState`、`DocumentView`、`NoteId`、`WorkspaceId` 或 `TagId`。
- `notora-app` 负责把 catalog、产品状态和 EditorRuntime 状态映射为 UI 输入。
- 标题编辑必须修改当前 EditorRuntime 中的同一个 `DocumentModel`，共享 undo、dirty、自动保存和插件刷新；禁止绕开 runtime 直接写文件。
- SQLite 和文件操作只允许在既有 action/effect/worker 边界后执行。
- 加密状态使用枚举，禁止使用多个布尔值组合表达。
- 每个实现子任务最多修改 3 个文件；发现超出时必须继续拆分。
- 每个行为变化必须先写失败测试，再做最小实现。
- 每个子任务提交前运行 `cargo fmt --all -- --check` 和相关 crate 的编译或测试。
- 每个阶段结束运行该阶段全部相关测试；最终运行 `./scripts/verify.sh`。
- 不把六种“文档类型 × 加密状态”组合展开成六个菜单项。
- 在真实加密保存链路完成前，不得展示可创建“已加密笔记”的入口，也不得只写一个 catalog 布尔值冒充加密。

## 4. 依赖顺序

```text
设计门槛
  ├─ 加密契约 ──> catalog 属性 ──> 加密创建/加载/保存
  └─ 标题展示规则 ──> 标题投影 ──> runtime 文本编辑协议

正式标签 core ──> metadata action/worker ──> 标签属性 UI
目录移动保存门槛 ────────────────────> 位置属性 UI

通用 UI 组件 + 编辑区子布局
  └─> Notora EditorPaneChrome 集成
       ├─> 标题/状态/属性
       ├─> 编辑器菜单
       └─> 新建笔记面板

全部集成 ──> 响应式与状态矩阵 ──> 全面验证
```

## 5. 阶段零：关闭设计门槛

### Task 0.1：确认 Mindmap 标题展示规则

**文件：**

- Modify: `docs/specs/2026-08-03-notora-editor-area-design.md`

**原因：**

Mindmap 的第一个 H1 同时是图中的根节点。若正文完全隐藏第一个 H1，会删除 Mindmap 的结构根；若继续显示，则会与头部标题重复。实施前必须明确例外规则。

**推荐决策：**

- Markdown 普通编辑模式由标题栏承载第一个 H1，正文不重复绘制；
- Mindmap 标题栏编辑同一个 H1，但画布仍显示根节点，因为它是结构元素；
- TXT 继续以第一行作为标题；
- 源码模式始终显示完整源码。

- [x] 写清 Mindmap 的展示例外与原因。
- [x] 增加 Markdown、Mindmap、TXT、源码模式四种验收规则。
- [x] 由产品决策确认后再开始标题渲染任务。

### Task 0.2：补齐真实加密设计

**文件：**

- Create: `docs/specs/2026-08-03-notora-note-encryption-design.md`

**必须确定：**

- 加密文件格式、版本头和损坏检测；
- 密钥来源、密钥保存、解锁生命周期和忘记密钥策略；
- 标题、标签、目录、时间和摘要哪些允许明文；
- 自动保存、dirty snapshot、冲突副本、catalog backup 和 FTS 不得泄漏哪些明文；
- 新建、加载、保存、外部修改、回收站和恢复路径如何编解码；
- 应用崩溃和保存失败时如何保证旧密文仍可恢复；
- 已有普通笔记不允许通过编辑区原地切换为加密；
- 加密笔记创建失败时不得遗留伪装成密文的明文文件或错误 catalog 状态。

- [x] 完成威胁边界和数据格式设计。
- [x] 指定使用经过审计的加密库，禁止自行实现密码学原语。
- [x] 给出测试向量、错误分类和密码不可用场景。
- [x] 已由 2026-08-20 加密笔记实施方案接续。

**门槛：** 加密创建选项必须等待创建、加载、保存、扫描、session 清理和 snapshot 隔离全部验收通过。

### Task 0.3：建立干净且可信的执行基线

**文件：** 无。

- [ ] 确认当前未提交修改由其原任务完成或安全隔离。
- [x] 运行 `cargo check -p notora-app`。
- [x] 运行 `cargo test -p notora-core`。
- [x] 运行 `cargo test -p textora-appkit-shell --lib`。
- [x] 记录任何与本计划无关的既有失败，不得通过放宽断言掩盖（本次全量验证无失败）。

## 6. 阶段一：catalog 与正式标签基础

### Task 1.1：增加创建时间与加密领域属性

**文件：**

- Modify: `crates/notora-core/src/domain.rs`
- Modify: `crates/notora-core/src/catalog/migration.rs`
- Modify: `crates/notora-core/src/catalog/note_repository.rs`

**接口：**

- 新增 `NoteEncryption::{Unencrypted, Encrypted}`；
- catalog schema 升级，`notes` 增加创建时间和加密属性；
- 旧库的创建时间使用现有 `modified_ns` 回填，加密属性回填为 `Unencrypted`；
- 新建 catalog 的完整 schema 直接包含新列；
- 提供只读 `NoteEditorMetadata` 查询，不把所有新字段强行塞进扫描器的 `CatalogNote`。

- [x] 先写 schema v3 升级、旧数据回填、非法加密值拒绝的失败测试。
- [x] 实现 migration 和数据库值到枚举的严格转换。
- [x] 写 `NoteEditorMetadata` 查询往返测试。
- [x] 运行 `cargo test -p notora-core catalog::migration`。
- [x] 运行 `cargo test -p notora-core catalog::note_repository`。
- [ ] 提交：`feat(notora): persist editor metadata`

### Task 1.2：让新建命令显式携带存储属性

**文件：**

- Modify: `crates/notora-core/src/note_command.rs`
- Modify: `crates/notora-core/src/lib.rs`

**接口：**

- 新增显式包含 `NoteEncryption` 的 configured create request；
- configured request 只表达类型、目录和存储属性，不包含从导航标签隐式附加标签的字段；
- 创建时间由创建命令生成一次并写入 catalog；
- 未接入加密引擎前，产品只能构造 `Unencrypted` 请求。
- 为保证每次提交可编译，先保留现有 `CreateNoteRequest` 作为临时兼容入口；Task 7.2 迁移产品调用方，Task 8.4 最终删除兼容入口。

- [x] 先写普通笔记创建后属性固定的失败测试。
- [x] 删除 `tag_to_attach` 创建捷径，避免标签导航隐式影响新建笔记。
- [x] 运行 `cargo test -p notora-core note_command`。
- [ ] 提交：`refactor(notora): type note creation properties`

### Task 1.3：验证层级标签名称

**文件：**

- Modify: `crates/notora-core/src/catalog/metadata_repository.rs`

**接口：**

- 将内部 `TagName` 收敛为可测试的层级标签解析；
- `/` 为唯一分隔符；
- 每一段 trim、NFC 规范化，禁止空段；
- 完整路径按现有大小写不敏感规则唯一；
- 新增“按展示名创建或复用并原子附加”的 catalog 方法。

- [x] 写中文、ASCII、多层、重复、空段、首尾 `/` 和 Unicode 规范化测试。
- [x] 写同名并发/重复附加保持幂等的事务测试。
- [x] 保持现有 `TagId` 稳定语义。
- [x] 运行 `cargo test -p notora-core catalog::metadata_repository`。
- [ ] 提交：`feat(notora): validate hierarchical tag paths`

### Task 1.4：停止正文 hashtag 覆盖正式标签

**文件：**

- Modify: `crates/notora-core/src/scan.rs`
- Modify: `crates/notora-core/src/lib.rs`
- Delete: `crates/notora-core/src/hashtags.rs`

- [x] 先把扫描测试改为：正文 hashtag 变化后，正式标签保持不变。
- [x] 删除扫描路径中的 `replace_note_tags` 调用。
- [x] 搜索索引继续读取 catalog 中的正式标签。
- [x] 删除不再使用的 hashtag 提取模块和公开导出。
- [x] 验证全量扫描、增量扫描和移动重建都不清空正式标签。
- [x] 运行 `cargo test -p notora-core scan`。
- [ ] 提交：`refactor(notora): make catalog tags authoritative`

### Task 1.5：增加正式标签 mutation

**文件：**

- Modify: `crates/notora-app/src/action.rs`
- Modify: `crates/notora-app/src/workspace_controller.rs`
- Modify: `crates/notora-app/src/product.rs`

**接口：**

- `MetadataMutation::AttachTagByName { note_id, display_name }`；
- `MetadataMutation::DetachTag { note_id, tag_id }`；
- worker 完成事件返回受影响 `note_id` 和最新 `NoteEditorMetadata`，而不是无信息的完成信号；
- 星标完成事件也返回最新 metadata。

- [x] 先写 worker 成功、重复附加、非法层级和 detach 幂等测试。
- [x] 保证 SQL 仍只在 index worker 中执行。
- [x] 运行 `cargo test -p notora-app workspace_controller`。
- [ ] 提交：`feat(notora): add typed tag metadata mutations`

## 7. 阶段二：标题投影与 EditorRuntime 协议

### Task 2.1：建立纯标题投影算法

**文件：**

- Modify: `crates/notora-core/src/summary_parser.rs`
- Modify: `crates/notora-core/src/lib.rs`

**接口：**

- 输入 `DocumentKind` 和完整源码；
- 输出标题文本、标题内容字节范围，以及没有标题时的插入位置；
- Markdown/Mindmap 识别第一个一级标题；
- TXT 识别第一行；
- 生成替换计划时负责 `# `、换行和空文档规范化。

- [x] 先写空文档、已有 H1、多 H1、中文标题、CRLF、TXT 第一行测试。
- [x] 写“编辑标题不改文件名”的领域边界测试。
- [x] 确保范围都是 UTF-8 字节范围，不按字符索引切片。
- [x] 运行 `cargo test -p notora-core summary_parser`。
- [ ] 提交：`feat(notora): project document title edits`

### Task 2.2：增加产品安全的文本快照与范围替换

**文件：**

- Modify: `crates/appkit-shell/src/editor_runtime/contract.rs`
- Modify: `crates/appkit-shell/src/editor_runtime/model_session.rs`
- Modify: `crates/appkit-shell/src/editor_runtime/mod.rs`

**接口：**

- `document_text_snapshot(tab_id)` 返回文本和 content revision；
- `replace_document_text(request)` 校验 tab、revision 和 UTF-8 字节边界；
- 成功替换走 DocumentModel undo group，并刷新插件 source、光标展示、dirty 和通知；
- 过期 revision 返回明确的拒绝结果，不覆盖用户并发输入。

- [x] 先写替换成功、过期 revision、非法范围、undo、dirty 和插件刷新测试。
- [x] 复用现有 `edit_outcome` 通知生成，避免标题编辑绕开自动保存。
- [x] 不向 Notora 暴露可任意持有的 `DocumentModel` 可变引用。
- [x] 运行 `cargo test -p textora-appkit-shell editor_runtime`。
- [ ] 提交：`feat(appkit): expose revision-checked text edits`

### Task 2.3：接入标题 action/effect

**文件：**

- Modify: `crates/notora-app/src/action.rs`
- Modify: `crates/notora-app/src/state.rs`
- Modify: `crates/notora-app/src/app.rs`

**接口：**

- `TitleCommitRequested(String)` 只对活动工作区笔记生效；
- effect 使用标题投影算法和 revision-checked runtime API；
- 成功结果通过现有 `EditorNotification` 进入自动保存；
- `Escape` 只撤销 TextBox 草稿，不产生文档编辑；
- 外部文件和回收站不允许通过该 action 修改标题。

- [x] 先写 reducer 非法状态拒绝测试。
- [x] 写 app 集成测试，证明标题提交产生 dirty、content revision 和自动保存计划。
- [x] 写过期 revision 不覆盖正文的测试。
- [x] 运行 `cargo test -p notora-app --lib title`。
- [ ] 提交：`feat(notora): edit note titles through runtime`

### Task 2.4：实现 Markdown 首个 H1 的展示策略

**文件：**

- Modify: `crates/markdown/src/view.rs`
- Modify: `crates/markdown/src/layout.rs`
- Modify: `crates/notora-app/src/editor_adapter.rs`

**前提：** Task 0.1 已确认。

- [x] 为 Notora Markdown 插件配置“标题由外部头部承载”。
- [x] 普通 Markdown 布局跳过第一个 H1 的可见块，但保持后续源码字节映射正确。
- [x] textora 产品默认配置不改变。
- [x] Mindmap 按已确认的根节点规则处理，不破坏树结构。
- [x] 写光标、选择、首段定位和源码模式回归测试。
- [x] 运行 `cargo test -p textora-markdown`。
- [x] 运行 `cargo test -p notora-app editor_adapter`。
- [ ] 提交：`feat(markdown): support externally rendered note titles`

## 8. 阶段三：通用 UI 组件

### Task 3.1：编辑区标题与状态组件

**文件：**

- Create: `crates/ui/src/widgets/editor_header.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

**纯数据输入：**

- 标题、是否可编辑；
- 创建、修改和保存状态文本；
- 星标状态与可操作性；
- `EncryptionStatusInput::{Unencrypted, Encrypted, Hidden}`；
- 删除操作可见性；
- 紧凑/完整展示模式。

- [x] 先写标题提交、Escape、星标、删除和只读加密命中测试。
- [x] 加密状态不得产生 action、焦点或按钮 hover。
- [x] 时间与状态文本必须按实际 shaping 宽度折叠。
- [x] 运行 `cargo test -p textora-ui editor_header`。
- [ ] 提交：`feat(ui): add editor document header`

### Task 3.2：位置选择组件

**文件：**

- Create: `crates/ui/src/widgets/location_picker.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

- [x] 输入只包含工作区展示名、当前相对路径和纯目录行。
- [x] 输出使用通用 row key，不携带 `WorkspaceId` 或 `PathBuf` 领域语义。
- [x] 支持根目录、展开、选择、Escape 和点击外部关闭。
- [x] 禁止选择不存在或禁用的目录行。
- [x] 运行 `cargo test -p textora-ui location_picker`。
- [ ] 提交：`feat(ui): add location picker widget`

### Task 3.3：层级标签编辑组件

**文件：**

- Create: `crates/ui/src/widgets/tag_editor.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

- [x] 输入为纯 `TagChipInput`、补全候选和 pending 状态。
- [x] 输出为提交文本、移除 chip key、展开和关闭，不携带 `TagId`。
- [x] 支持 Enter、Backspace、Escape、补全选择和单行 `+N` 折叠。
- [x] 组件只做输入体验，不实现标签路径领域校验。
- [x] 运行 `cargo test -p textora-ui tag_editor`。
- [ ] 提交：`feat(ui): add hierarchical tag editor`

### Task 3.4：编辑器菜单组件

**文件：**

- Create: `crates/ui/src/widgets/editor_toolbar.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

- [x] 输入是通用命令项、分组、选中态、启用态和 overflow 优先级。
- [x] 输出只包含稳定 `EditorToolbarCommandKey`。
- [x] 窄宽度把低优先级命令移入“更多”，不缩小命中区。
- [x] TXT、Markdown 和 Mindmap 的命令集合由产品层注入，widget 不识别文档类型。
- [x] 运行 `cargo test -p textora-ui editor_toolbar`。
- [ ] 提交：`feat(ui): add responsive editor toolbar`

### Task 3.5：新建笔记面板组件

**文件：**

- Create: `crates/ui/src/widgets/note_creation_panel.rs`
- Modify: `crates/ui/src/widgets/mod.rs`
- Modify: `crates/ui/src/lib.rs`

- [x] 输入为文档类型选项、目录选项、存储方式选项和提交状态。
- [x] 输出使用通用 option key，不携带 `DocumentKind`、`PathBuf` 或 `NoteEncryption`。
- [x] 支持键盘导航、Escape、确认和失败状态。
- [x] 没有加密引擎时，产品层只注入普通存储选项。
- [x] 运行 `cargo test -p textora-ui note_creation_panel`。
- [ ] 提交：`feat(ui): add note creation panel`

## 9. 阶段四：编辑区布局与 Notora 组合

### Task 4.1：拆分编辑区几何

**文件：**

- Modify: `crates/notora-app/src/shell/layout.rs`
- Modify: `crates/notora-app/src/events.rs`

**输出矩形：**

- `editor_header_rect`；
- `editor_toolbar_rect`；
- `editor_body_rect`；
- 属性弹层仍使用 overlay/menu 层，不侵入正文矩形。

- [x] 先写三栏、导航覆盖、编辑器覆盖、高 DPI 和最小高度测试。
- [x] EditorRuntime 的鼠标、滚轮、IME 和绘制只使用 `editor_body_rect`。
- [x] 文档头部或菜单事件不得落入正文。
- [x] 运行 `cargo test -p notora-app shell::layout`。
- [x] 运行 `cargo test -p notora-app events`。
- [ ] 提交：`refactor(notora): split editor chrome geometry`

### Task 4.2：建立 Notora 编辑区组合器

**文件：**

- Create: `crates/notora-app/src/editor_pane.rs`
- Modify: `crates/notora-app/src/lib.rs`
- Modify: `crates/notora-app/src/render.rs`

**职责：**

- `EditorPaneChrome` 组合 header、location picker、tag editor 和 toolbar；
- `EditorPaneInput` 是由 app 层构造的纯展示模型；
- `render.rs` 不继续堆叠头部命中测试和绘制细节；
- 原中栏笔记工具按钮迁出，只保留列表级动作。

- [x] 先写普通、外部、回收站和空状态输入矩阵测试。
- [x] 写所有弹层关闭后不可继续命中的测试。
- [x] 运行 `cargo test -p notora-app editor_pane`。
- [x] 运行 `cargo test -p notora-app render`。
- [ ] 提交：`refactor(notora): compose editor pane chrome`

### Task 4.3：加载并维护活动笔记 metadata

**文件：**

- Modify: `crates/notora-app/src/product.rs`
- Modify: `crates/notora-app/src/workspace_controller.rs`
- Modify: `crates/notora-app/src/action.rs`

- [x] 工作区文档加载结果同时携带 `NoteEditorMetadata` 和正式标签。
- [x] metadata mutation 完成结果携带最新属性和标签快照。
- [x] 定义带 identity/generation 的 metadata action，供 reducer 拒绝过期结果。
- [x] SQL 查询仍只发生在 workspace worker。
- [x] 运行 `cargo test -p notora-app workspace_controller`。
- [ ] 提交：`feat(notora): track active editor metadata`

### Task 4.4：构造编辑区展示模型和保存状态

**文件：**

- Modify: `crates/notora-app/src/app.rs`
- Modify: `crates/notora-app/src/render.rs`
- Modify: `crates/notora-app/src/state.rs`

- [x] `LibraryState` 使用带 identity/generation 的加载状态，拒绝 A→B→A 的过期结果。
- [x] metadata mutation、移动、保存后刷新当前笔记 metadata。
- [x] 外部文件状态不得残留上一篇工作区笔记属性。
- [x] app 将 metadata、runtime dirty/revision 和现有 autosave 状态映射为 `EditorPaneInput`。
- [x] 保存状态使用枚举映射为保存中、已保存和保存失败。
- [x] 创建/修改时间格式化集中在纯函数中，创建时间显示 UTC 日期、修改时间显示相对时间。
- [x] 加密只映射为只读状态，不生成 action。
- [x] EditorRuntime 绘制和输入改用 `editor_body_rect`。
- [x] 运行 `cargo test -p notora-app state`。
- [x] 运行 `cargo test -p notora-app render`。
- [ ] 提交：`feat(notora): render editor document state`

### Task 4.5：接入标题、星标、标签和删除事件

**文件：**

- Modify: `crates/notora-app/src/editor_pane.rs`
- Modify: `crates/notora-app/src/action.rs`
- Modify: `crates/notora-app/src/state.rs`

- [x] 将通用 widget action 映射为类型化 `NotoraAction`。
- [x] 加密状态不建立任何 action 分支。
- [x] 回收站只映射恢复和永久删除；外部文件隐藏不合法动作。
- [x] 标签提交在 pending 期间去重，完成后用最新标签快照刷新 chip，失败后恢复可提交并显示错误。
- [x] 运行 `cargo test -p notora-app editor_pane`。
- [x] 运行 `cargo test -p notora-app state`。
- [ ] 提交：`feat(notora): route editor chrome actions`

## 10. 阶段五：位置移动的保存门槛

### Task 5.1：移动 dirty 笔记前强制保存

**文件：**

- Modify: `crates/notora-app/src/app.rs`

现有普通移动会直接执行文件命令；本任务复用回收站已有的“先保存、再移动”思路，但使用独立的互斥 pending 状态。

- [x] 先写 dirty 笔记选择目录后不会立即移动的失败测试。
- [x] 保存成功且 revision 匹配后才执行 `MoveNoteRequest`。
- [x] 保存失败、产生冲突或 revision 已变化时取消移动并保留原路径。
- [x] clean 笔记直接进入现有移动命令。
- [x] 同名目标继续返回现有 `TargetAlreadyExists`，不得覆盖。
- [x] 移动成功后更新打开 tab 路径并保持 `NoteId`、光标和 tab。
- [x] 运行 `cargo test -p notora-app --lib move`。
- [ ] 提交：`fix(notora): save dirty notes before moving`

### Task 5.2：接入位置选择器

**文件：**

- Modify: `crates/notora-app/src/editor_pane.rs`
- Modify: `crates/notora-app/src/render.rs`
- Modify: `crates/notora-app/src/state.rs`

- [x] 从当前导航目录快照构造 picker 输入和 row key→相对路径映射。
- [x] 根目录使用明确的空相对路径，不使用特殊字符串魔法值。
- [x] 只允许活动工作区目录；回收站和外部文件隐藏控件。
- [x] 选择当前目录是幂等操作。
- [x] 运行 `cargo test -p notora-app editor_pane`。
- [x] 运行 `cargo test -p notora-app state`。
- [ ] 提交：`feat(notora): select note locations in editor`

## 11. 阶段六：编辑器菜单命令

### Task 6.1：扩展语义编辑命令协议

**文件：**

- Modify: `crates/ui/src/plugin.rs`
- Modify: `crates/appkit-shell/src/editor_runtime/model_session.rs`
- Modify: `crates/appkit-shell/src/editor_runtime/mod.rs`

**新增语义命令：**

- Undo、Redo；
- 设置标题级别；
- 切换粗体、斜体、删除线、行内代码；
- 项目列表、编号列表、任务列表、引用、代码块；
- 插入链接。

- [x] 不支持某命令的插件必须返回 typed unsupported，不得静默修改源码。
- [x] 所有成功命令生成与键盘编辑一致的 dirty、revision 和通知。
- [x] 运行 `cargo test -p textora-appkit-shell editor_runtime`。
- [ ] 提交：`feat(appkit): execute semantic editor commands`

### Task 6.2：实现 Markdown 命令转换

**文件：**

- Create: `crates/markdown/src/commands.rs`
- Modify: `crates/markdown/src/lib.rs`
- Modify: `crates/markdown/src/view.rs`

- [x] 先写无选择、单行选择、多行选择、嵌套标记和中文范围测试。
- [x] 命令产生 `EditTransaction`，不得直接写 `TextBuffer`。
- [x] 再次执行 toggle 命令可以去除对应标记。
- [x] 标题命令不得把 Notora 的唯一标题 H1 误改成正文普通标题。
- [x] 运行 `cargo test -p textora-markdown commands`。
- [ ] 提交：`feat(markdown): resolve toolbar formatting commands`

### Task 6.3：接入按类型变化的菜单

**文件：**

- Modify: `crates/notora-app/src/editor_pane.rs`
- Modify: `crates/notora-app/src/app.rs`
- Modify: `crates/notora-app/src/render.rs`

- [x] TXT 只展示运行时实际支持的通用命令。
- [x] Markdown 展示格式化命令。
- [x] Mindmap 展示节点层级、展开和收起等实际支持命令。
- [x] 禁用态与 unsupported 结果保持一致。
- [x] 菜单命令执行后焦点回到正文。
- [x] 运行 `cargo test -p notora-app editor_pane`。
- [ ] 提交：`feat(notora): connect document-specific editor menus`

## 12. 阶段七：新建笔记面板与加密接入

### Task 7.1：建立创建面板状态机

**文件：**

- Modify: `crates/notora-app/src/action.rs`
- Modify: `crates/notora-app/src/state.rs`
- Modify: `crates/notora-app/src/render.rs`

- [x] 用 `NewNoteDraft` 表达类型、目录、存储方式和提交状态。
- [x] `OverlayState::NewDocumentMenu` 收敛为互斥的创建面板状态。
- [x] 普通按钮可以使用默认类型打开面板，不直接跳过位置和存储方式确认。
- [x] Trash 状态不能打开创建面板。
- [x] Escape 取消草稿，不创建文件。
- [x] 运行 `cargo test -p notora-app state`。
- [x] 运行 `cargo test -p notora-app render`。
- [ ] 提交：`feat(notora): model note creation drafts`

### Task 7.2：接入普通笔记创建

**文件：**

- Modify: `crates/notora-app/src/editor_pane.rs`
- Modify: `crates/notora-app/src/effect_executor.rs`
- Modify: `crates/notora-app/src/app.rs`

- [x] 类型、当前工作区目录和 `Unencrypted` 完整进入 `ConfiguredCreateNoteRequest`。
- [x] 创建成功后选中并打开新笔记，标题栏进入可编辑状态。
- [x] 创建失败保留面板草稿并显示错误。
- [x] 删除旧的三项简单菜单路径和失效命中矩形。
- [x] 运行 `cargo test -p notora-app --lib create`。
- [ ] 提交：`feat(notora): create notes from creation panel`

### Task 7.3：接入真实加密创建（已被替代）

本任务已由 2026-08-20 加密笔记实施方案替代。创建、解锁、保存、扫描、快照隔离、冲突和生命周期必须按新方案整体实施与验收，不能从本历史任务中单独启用加密入口。

## 13. 阶段八：响应式、状态矩阵与收尾

### Task 8.1：响应式头部与折叠规则

**文件：**

- Modify: `crates/ui/src/widgets/editor_header.rs`
- Modify: `crates/ui/src/widgets/editor_toolbar.rs`
- Modify: `crates/notora-app/src/editor_pane.rs`

- [x] 宽模式展示完整时间、位置、标签和常用命令。
- [x] 中等宽度缩短时间，标签折叠为 `+N`，低频命令进入“更多”。
- [x] 最窄模式保留标题、保存状态、星标和加密状态。
- [x] 删除进入“更多”，加密状态仍不可点击。
- [x] 高 DPI 和最小窗口不发生重叠或负矩形。
- [x] 运行相关 UI 与 Notora 响应式测试。
- [ ] 提交：`feat(notora): adapt editor chrome responsively`

### Task 8.2：完整文档状态矩阵

**文件：**

- Modify: `crates/notora-app/src/editor_pane.rs`
- Modify: `crates/notora-app/src/render.rs`
- Modify: `crates/notora-app/src/events.rs`

- [x] 普通笔记：完整可编辑头部。
- [x] 外部文件：文件名、路径和手动保存；隐藏位置、标签、星标和加密。
- [x] 回收站：只读标题/正文，提供恢复和永久删除。
- [x] 空状态：不保留任何可操作的旧笔记命中区域。
- [x] 切换文档后关闭旧弹层和 pending 输入。
- [x] 模态层打开时阻断标题、属性、菜单和正文输入。
- [ ] 提交：`test(notora): cover editor pane state matrix`

### Task 8.3：删除旧路径与文档同步

**文件：**

- Modify: `crates/notora-app/src/render.rs`
- Modify: `docs/specs/2026-08-03-notora-layout-localization-content-tags-design.md`
- Modify: `docs/specs/2026-08-03-notora-editor-area-design.md`

- [x] 删除中栏笔记级重命名、移动、星标和回收站旧工具栏状态。
- [x] 删除旧的新建三项菜单常量、布局和命中函数。
- [x] 更新旧标签规格，直接标注其标签章节已被替代，不再只依赖新文档中的间接说明。
- [x] 根据最终实现同步状态矩阵和加密依赖说明。
- [x] 运行 `rg` 确认没有遗留正文 hashtag 覆盖链路和加密切换 action。
- [ ] 提交：`refactor(notora): remove superseded editor chrome paths`

### Task 8.4：删除新建命令兼容入口

**文件：**

- Modify: `crates/notora-core/src/note_command.rs`
- Modify: `crates/notora-app/src/effect_executor.rs`
- Modify: `crates/notora-app/src/workspace_controller.rs`

- [x] 所有生产调用和测试夹具改用 configured create request。
- [x] 删除旧 `CreateNoteRequest`、`tag_to_attach` 和兼容 `NoteCommand` variant。
- [x] catalog 新建路径不再接受隐式标签关联。
- [x] 运行 `cargo test -p notora-core note_command`。
- [x] 运行 `cargo test -p notora-app effect_executor`。
- [x] 运行 `cargo test -p notora-app workspace_controller`。
- [ ] 提交：`refactor(notora): remove legacy note creation request`

## 14. 阶段验证

### 领域与 catalog

- [x] `cargo fmt --all -- --check`
- [x] `cargo test -p notora-core`
- [x] `cargo check -p notora-core`

### EditorRuntime 与 Markdown

- [x] `cargo test -p textora-appkit-shell --lib`
- [x] `cargo test -p textora-markdown`
- [x] `cargo check -p textora-appkit-shell`

### UI 与 Notora

- [x] `cargo test -p textora-ui`
- [x] `cargo test -p notora-app --lib`
- [x] `cargo check -p notora-app`

### 最终全面验证

- [x] `./scripts/verify.sh`
- [ ] 启动 Notora，人工验证普通 Markdown、TXT、Mindmap、外部文件、回收站和空状态。
- [ ] 人工验证标题提交、Escape、undo、自动保存、保存失败和外部 H1 修改。
- [ ] 人工验证目录移动、同名冲突、dirty 保存失败和移动后 tab 保持。
- [ ] 人工验证层级标签补全、移除、重扫保留和搜索结果刷新。
- [ ] 人工验证加密状态不可点击、普通存储文件内容正确且加密创建入口保持隐藏；真实加密结果另行验收。
- [ ] 人工验证最小窗口、三栏、导航覆盖、编辑器覆盖和 2x DPI。

## 15. 完成定义

只有同时满足以下条件，本计划才算完成：

- 已确认规格中的所有编辑区区域均落地；
- 标题编辑与正文共享同一个 undo、dirty 和自动保存链路；
- 正式标签不再被正文 hashtag 或工作区扫描覆盖；
- 目录选择只作用于当前活动工作区，dirty 移动不会丢内容；
- 加密状态只读，且“已加密”一定对应真实加密存储；
- 普通、外部、回收站和空状态没有越权操作；
- 每个子任务不超过 3 个修改文件并拥有独立可编译提交；
- `./scripts/verify.sh` 全部通过；
- 规格、迁移说明和最终行为一致。
